use crate::coords::{parse_coord_query, validate_coords};
use crate::i18n::Locale;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub os_version: String,
    pub transport: String, // "wifi", "usb", "usb+wifi"
    pub can_devicectl: bool,
    pub can_userspace: bool,
    pub can_remote_pairing: bool,
    pub remote_identifier: String,
}

#[derive(Clone, Debug, Default)]
pub struct SpooferState {
    pub busy: bool,
    pub active: bool,
    pub udid: String,
    pub latitude: f64,
    pub longitude: f64,
    pub label: String,
    pub method: String, // "devicectl", "remotepairing", "pymobiledevice3"
    pub message: String,
    pub error: String,
    pub logs: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PairingState {
    pub active: bool,
    pub host_name: String,
    pub status: String, // "idle", "waiting", "pin", "paired", "error"
    pub pin: String,
    pub device_name: String,
    pub device_udid: String,
    pub message: String,
    pub error: String,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub name: String,
    pub sub: String,
    pub latitude: f64,
    pub longitude: f64,
    pub direct: bool,
}

#[derive(Deserialize)]
struct UsbmuxEntry {
    #[serde(rename = "Identifier", default)]
    identifier: String,
    #[serde(rename = "UniqueDeviceID", default)]
    unique_device_id: String,
    #[serde(rename = "DeviceName", default)]
    device_name: String,
    #[serde(rename = "ProductType", default)]
    product_type: String,
    #[serde(rename = "ProductVersion", default)]
    product_version: String,
    #[serde(rename = "ConnectionType", default)]
    connection_type: String,
}

#[derive(Clone, Debug)]
struct DevicectlEntry {
    udid: String,
    name: String,
    model: String,
    os_version: String,
    connected: bool,
    transport: String,
}

#[derive(Deserialize)]
struct RemoteWifiEntry {
    #[serde(default)]
    identifier: String,
}

struct ActiveProc {
    pid: u32,
    done: Arc<Mutex<bool>>,
}

struct Inner {
    state: SpooferState,
    devices: Vec<Device>,
    device_map: HashMap<String, Device>,
    active_proc: Option<ActiveProc>,
    active_id: u64,
    pair_state: PairingState,
    pair_proc: Option<ActiveProc>,
    pair_id: u64,
    locale: Locale,
}

#[derive(Clone)]
pub struct SpooferController {
    inner: Arc<Mutex<Inner>>,
    version: Arc<AtomicU64>,
}

impl SpooferController {
    pub fn new(locale: Locale) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: SpooferState::default(),
                devices: Vec::new(),
                device_map: HashMap::new(),
                active_proc: None,
                active_id: 0,
                pair_state: PairingState {
                    status: "idle".into(),
                    ..Default::default()
                },
                pair_proc: None,
                pair_id: 0,
                locale,
            })),
            version: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    pub fn set_locale(&self, locale: Locale) {
        self.inner.lock().unwrap().locale = locale;
        self.bump();
    }

    pub fn snapshot(&self) -> (SpooferState, Vec<Device>, PairingState) {
        let g = self.inner.lock().unwrap();
        (g.state.clone(), g.devices.clone(), g.pair_state.clone())
    }

    fn append_log_locked(g: &mut Inner, line: impl AsRef<str>) {
        let s = line.as_ref().trim();
        if s.is_empty() {
            return;
        }
        let ts = Local::now().format("%H:%M:%S");
        g.state.logs.push(format!("[{ts}] {s}"));
        if g.state.logs.len() > 80 {
            let excess = g.state.logs.len() - 80;
            g.state.logs.drain(0..excess);
        }
    }

    /// Spawn a background 3-way device scan (RemotePairing + usbmux + devicectl).
    pub fn scan_devices_async(&self) {
        let ctrl = self.clone();
        thread::spawn(move || {
            let h1 = thread::spawn(query_usbmux);
            let h2 = thread::spawn(query_devicectl);
            let h3 = thread::spawn(query_remote_browse);

            let usbmux = h1.join().unwrap_or_else(|_| Err("usbmux thread panic".into()));
            let devctl = h2.join().unwrap_or_else(|_| Err("devicectl thread panic".into()));
            let remote = h3.join().unwrap_or_else(|_| Err("remote browse thread panic".into()));

            let usbmux_list = usbmux.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
            let devctl_list = devctl.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
            let remote_list = remote.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);

            let merged = merge_devices(usbmux_list, devctl_list, remote_list);

            {
                let mut g = ctrl.inner.lock().unwrap();
                let locale = g.locale;
                if let Err(e) = &usbmux {
                    Self::append_log_locked(&mut g, format!("usbmux: {e}"));
                }
                if let Err(e) = &remote {
                    Self::append_log_locked(&mut g, format!("RemotePairing: {e}"));
                }
                if let Err(e) = &devctl {
                    Self::append_log_locked(&mut g, format!("devicectl: {e}"));
                }
                let summary = match locale {
                    Locale::ZhHans => format!(
                        "扫描完成：发现 {} 台设备（RemotePairing:{}, usbmux:{}）",
                        merged.len(),
                        remote_list.len(),
                        usbmux_list.len()
                    ),
                    Locale::En => format!(
                        "Scan finished: {} device(s) (RemotePairing:{}, usbmux:{})",
                        merged.len(),
                        remote_list.len(),
                        usbmux_list.len()
                    ),
                };
                Self::append_log_locked(&mut g, summary);
                g.device_map = merged.iter().map(|d| (d.udid.clone(), d.clone())).collect();
                g.devices = merged;
            }
            ctrl.bump();
        });
    }

    /// Lock a WGS-84 location on the target iOS device in a background thread.
    pub fn set_location_async(&self, udid: String, lat: f64, lng: f64, label: String) {
        let ctrl = self.clone();
        thread::spawn(move || {
            let locale = ctrl.inner.lock().unwrap().locale;
            if let Err(e) = validate_coords(lat, lng) {
                let mut g = ctrl.inner.lock().unwrap();
                g.state.error = e;
                ctrl.bump();
                return;
            }
            let udid = udid.trim().to_string();
            if udid.is_empty() {
                let mut g = ctrl.inner.lock().unwrap();
                g.state.error = locale.err_no_device().into();
                ctrl.bump();
                return;
            }

            let dev = {
                let g = ctrl.inner.lock().unwrap();
                g.device_map.get(&udid).cloned().unwrap_or(Device {
                    udid: udid.clone(),
                    name: udid.clone(),
                    model: String::new(),
                    os_version: String::new(),
                    transport: "wifi".into(),
                    can_devicectl: false,
                    can_userspace: true,
                    can_remote_pairing: false,
                    remote_identifier: String::new(),
                })
            };

            let label = if label.trim().is_empty() {
                format!("{lat:.5}, {lng:.5}")
            } else {
                label.trim().to_string()
            };

            let was_running = ctrl.stop_active_process();

            {
                let mut g = ctrl.inner.lock().unwrap();
                g.state.busy = true;
                g.state.error.clear();
                g.state.message = locale.status_syncing_detail().into();
                Self::append_log_locked(&mut g, format!("Set -> {label} ({lat:.6}, {lng:.6})"));
            }
            ctrl.bump();

            // Give iOS a brief cooldown to release its single RemotePairing tunnel slot.
            if was_running && dev.can_remote_pairing {
                thread::sleep(Duration::from_millis(1500));
            }

            if dev.can_devicectl {
                match run_devicectl_set(&udid, lat, lng) {
                    Ok(()) => {
                        let mut g = ctrl.inner.lock().unwrap();
                        g.state.busy = false;
                        g.state.active = true;
                        g.state.udid = udid;
                        g.state.latitude = lat;
                        g.state.longitude = lng;
                        g.state.label = label;
                        g.state.method = "devicectl".into();
                        g.state.error.clear();
                        g.state.message = locale.status_active_devicectl().into();
                        Self::append_log_locked(&mut g, "devicectl set OK");
                        ctrl.bump();
                        return;
                    }
                    Err(e) => {
                        let mut g = ctrl.inner.lock().unwrap();
                        Self::append_log_locked(&mut g, format!("devicectl fallback: {e}"));
                    }
                }
            }

            let use_remote = dev.can_remote_pairing;
            let target_id = if use_remote && !dev.remote_identifier.is_empty() {
                dev.remote_identifier.clone()
            } else {
                udid.clone()
            };

            let mut res = ctrl.start_pymobiledevice3_set(
                &udid,
                &target_id,
                use_remote,
                lat,
                lng,
                &label,
                locale,
            );
            if let Err(ref e) = res {
                if use_remote && e.contains("RemotePairing") {
                    thread::sleep(Duration::from_secs(2));
                    res = ctrl.start_pymobiledevice3_set(
                        &udid, &target_id, true, lat, lng, &label, locale,
                    );
                }
            }

            if let Err(e) = res {
                let mut g = ctrl.inner.lock().unwrap();
                g.state.busy = false;
                g.state.active = false;
                g.state.error = e.clone();
                g.state.message = locale.status_set_failed().into();
                Self::append_log_locked(&mut g, format!("Set failed: {e}"));
                ctrl.bump();
            }
        });
    }

    /// Clear simulated location & restore real GPS on the target iOS device.
    pub fn clear_location_async(&self, udid: String) {
        let ctrl = self.clone();
        thread::spawn(move || {
            let (locale, resolved_udid, dev) = {
                let mut g = ctrl.inner.lock().unwrap();
                let u = if udid.trim().is_empty() {
                    g.state.udid.clone()
                } else {
                    udid.trim().to_string()
                };
                let d = g.device_map.get(&u).cloned();
                let loc = g.locale;
                g.state.busy = true;
                g.state.error.clear();
                g.state.message = loc.status_clearing().into();
                Self::append_log_locked(&mut g, loc.status_clearing());
                (loc, u, d)
            };
            ctrl.bump();

            let was_running = ctrl.stop_active_process();
            let can_remote = dev.as_ref().map(|d| d.can_remote_pairing).unwrap_or(false);
            let can_devicectl = dev.as_ref().map(|d| d.can_devicectl).unwrap_or(false);

            if was_running && can_remote {
                thread::sleep(Duration::from_millis(1500));
            }

            let mut clear_err: Option<String> = None;
            if !resolved_udid.is_empty() {
                if can_devicectl {
                    if let Err(e) = run_devicectl_clear(&resolved_udid) {
                        clear_err = Some(e);
                    }
                }
                if !can_devicectl || clear_err.is_some() {
                    let target_id = dev
                        .as_ref()
                        .filter(|d| d.can_remote_pairing && !d.remote_identifier.is_empty())
                        .map(|d| d.remote_identifier.clone())
                        .unwrap_or_else(|| resolved_udid.clone());

                    let mut res = run_pymobiledevice3_clear(&target_id, can_remote, locale);
                    if let Err(ref e) = res {
                        if can_remote && e.contains("RemotePairing") {
                            thread::sleep(Duration::from_secs(2));
                            res = run_pymobiledevice3_clear(&target_id, true, locale);
                        }
                    }
                    clear_err = res.err();
                }
            }

            {
                let mut g = ctrl.inner.lock().unwrap();
                g.state.busy = false;
                g.state.active = false;
                g.state.label.clear();
                g.state.method.clear();
                if let Some(e) = clear_err {
                    g.state.error = e.clone();
                    g.state.message = match locale {
                        Locale::ZhHans => format!("已停止会话（clear 提示: {e}）"),
                        Locale::En => format!("Session stopped (clear error: {e})"),
                    };
                    Self::append_log_locked(&mut g, format!("Clear error: {e}"));
                } else {
                    g.state.error.clear();
                    g.state.message = locale.status_cleared().into();
                    Self::append_log_locked(&mut g, locale.status_cleared());
                }
            }
            ctrl.bump();
        });
    }

    pub fn start_wifi_pairing(&self, host_name: &str) {
        self.stop_wifi_pairing();
        let ctrl = self.clone();
        let host = if host_name.trim().is_empty() {
            "MacBook-Location".to_string()
        } else {
            host_name.trim().to_string()
        };

        thread::spawn(move || {
            let locale = ctrl.inner.lock().unwrap().locale;
            let (bin, prefix) = match find_pymobiledevice3(locale) {
                Ok(v) => v,
                Err(e) => {
                    let mut g = ctrl.inner.lock().unwrap();
                    g.pair_state.status = "error".into();
                    g.pair_state.error = e;
                    ctrl.bump();
                    return;
                }
            };

            let mut cmd = Command::new(&bin);
            cmd.args(&prefix)
                .args(["remote", "pair-host", "--name", &host, "--timeout", "300"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            apply_augmented_env(&mut cmd);

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let mut g = ctrl.inner.lock().unwrap();
                    g.pair_state.status = "error".into();
                    g.pair_state.error = format!("{e}");
                    ctrl.bump();
                    return;
                }
            };

            let pid = child.id();
            let done = Arc::new(Mutex::new(false));
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let my_id = {
                let mut g = ctrl.inner.lock().unwrap();
                g.pair_id += 1;
                let id = g.pair_id;
                g.pair_proc = Some(ActiveProc {
                    pid,
                    done: Arc::clone(&done),
                });
                g.pair_state = PairingState {
                    active: true,
                    host_name: host.clone(),
                    status: "waiting".into(),
                    message: locale.pair_waiting(&host),
                    ..Default::default()
                };
                Self::append_log_locked(&mut g, format!("Wi-Fi pair-host started: {host}"));
                id
            };
            ctrl.bump();

            let spawn_reader = |reader: Box<dyn std::io::Read + Send>, c: SpooferController| {
                thread::spawn(move || {
                    for line in BufReader::new(reader).lines().flatten() {
                        let line = line.trim().to_string();
                        if line.is_empty() {
                            continue;
                        }
                        let mut changed = false;
                        let mut paired_now = false;
                        {
                            let mut g = c.inner.lock().unwrap();
                            if g.pair_id != my_id {
                                return;
                            }
                            let loc = g.locale;
                            Self::append_log_locked(&mut g, format!("[pair] {line}"));
                            if let Some(pin) = parse_pair_pin(&line) {
                                g.pair_state.pin = pin;
                                g.pair_state.status = "pin".into();
                                g.pair_state.message = loc.pair_enter_pin().into();
                                changed = true;
                            } else if let Some(rest) = line.strip_prefix("name:") {
                                g.pair_state.device_name = rest.trim().into();
                                changed = true;
                            } else if let Some(rest) = line.strip_prefix("udid:") {
                                g.pair_state.device_udid = rest.trim().into();
                                changed = true;
                            } else if line.contains("Pairing record written to") {
                                g.pair_state.status = "paired".into();
                                g.pair_state.active = false;
                                g.pair_state.message = loc.pair_success().into();
                                changed = true;
                                paired_now = true;
                            }
                        }
                        if changed {
                            c.bump();
                        }
                        if paired_now {
                            c.scan_devices_async();
                        }
                    }
                });
            };

            if let Some(out) = stdout {
                spawn_reader(Box::new(out), ctrl.clone());
            }
            if let Some(err) = stderr {
                spawn_reader(Box::new(err), ctrl.clone());
            }

            let wait_res = child.wait();
            *done.lock().unwrap() = true;
            {
                let mut g = ctrl.inner.lock().unwrap();
                if g.pair_id == my_id {
                    g.pair_proc = None;
                    g.pair_state.active = false;
                    if g.pair_state.status != "paired" {
                        let loc = g.locale;
                        g.pair_state.status = "error".into();
                        g.pair_state.error = match wait_res {
                            Ok(st) if st.success() => loc.pair_timeout().into(),
                            _ => loc.pair_timeout().into(),
                        };
                    }
                }
            }
            ctrl.bump();
        });
    }

    pub fn stop_wifi_pairing(&self) {
        let proc_opt = {
            let mut g = self.inner.lock().unwrap();
            g.pair_id += 1;
            g.pair_state = PairingState {
                status: "idle".into(),
                ..Default::default()
            };
            g.pair_proc.take()
        };
        if let Some(p) = proc_opt {
            signal_and_wait_pid(p.pid, &p.done, Duration::from_millis(1500));
        }
        self.bump();
    }

    pub fn shutdown(&self) {
        self.stop_active_process();
        self.stop_wifi_pairing();
    }

    fn stop_active_process(&self) -> bool {
        let proc_opt = {
            let mut g = self.inner.lock().unwrap();
            g.active_id += 1;
            g.active_proc.take()
        };
        if let Some(p) = proc_opt {
            signal_and_wait_pid(p.pid, &p.done, Duration::from_millis(2500));
            return true;
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn start_pymobiledevice3_set(
        &self,
        udid: &str,
        target_id: &str,
        use_remote_pairing: bool,
        lat: f64,
        lng: f64,
        label: &str,
        locale: Locale,
    ) -> Result<(), String> {
        let (bin, prefix) = find_pymobiledevice3(locale)?;
        let (mode_flag, method_name, msg_text) = if use_remote_pairing {
            (
                "--remotepairing",
                "remotepairing",
                locale.status_active_remote(),
            )
        } else {
            (
                "--userspace",
                "pymobiledevice3",
                locale.status_active_userspace(),
            )
        };

        let mut cmd = Command::new(&bin);
        cmd.args(&prefix)
            .args([
                "developer",
                "dvt",
                "simulate-location",
                "set",
                mode_flag,
                "--udid",
                target_id,
                "--",
                &format!("{lat:.7}"),
                &format!("{lng:.7}"),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        apply_augmented_env(&mut cmd);

        let mut child: Child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
        let pid = child.id();
        let done = Arc::new(Mutex::new(false));
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let my_id = {
            let mut g = self.inner.lock().unwrap();
            g.active_id += 1;
            let id = g.active_id;
            g.active_proc = Some(ActiveProc {
                pid,
                done: Arc::clone(&done),
            });
            id
        };

        let ready = Arc::new(Mutex::new(false));
        let last_lines = Arc::new(Mutex::new(Vec::<String>::new()));

        let spawn_pipe = |reader: Box<dyn std::io::Read + Send>,
                          ctrl: SpooferController,
                          ready_cl: Arc<Mutex<bool>>,
                          lines_cl: Arc<Mutex<Vec<String>>>| {
            thread::spawn(move || {
                for line in BufReader::new(reader).lines().flatten() {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    {
                        let mut ll = lines_cl.lock().unwrap();
                        ll.push(line.clone());
                        if ll.len() > 10 {
                            ll.remove(0);
                        }
                    }
                    {
                        let mut g = ctrl.inner.lock().unwrap();
                        if g.active_id == my_id {
                            Self::append_log_locked(&mut g, &line);
                        }
                    }
                    if line.contains("Press Ctrl+C") || line.contains("SIGINT") {
                        *ready_cl.lock().unwrap() = true;
                    }
                }
            });
        };

        if let Some(out) = stdout {
            spawn_pipe(
                Box::new(out),
                self.clone(),
                Arc::clone(&ready),
                Arc::clone(&last_lines),
            );
        }
        if let Some(err) = stderr {
            spawn_pipe(
                Box::new(err),
                self.clone(),
                Arc::clone(&ready),
                Arc::clone(&last_lines),
            );
        }

        let ctrl_wait = self.clone();
        let done_wait = Arc::clone(&done);
        let exit_err = Arc::new(Mutex::new(None::<String>));
        let exit_err_cl = Arc::clone(&exit_err);
        thread::spawn(move || {
            let res = child.wait();
            *done_wait.lock().unwrap() = true;
            if let Ok(st) = &res {
                if !st.success() {
                    *exit_err_cl.lock().unwrap() = Some(format!("exit status {st}"));
                }
            } else if let Err(e) = &res {
                *exit_err_cl.lock().unwrap() = Some(e.to_string());
            }
            let mut g = ctrl_wait.inner.lock().unwrap();
            if g.active_id == my_id {
                g.active_proc = None;
                if g.state.active {
                    let loc = g.locale;
                    g.state.active = false;
                    g.state.message = loc.status_disconnected().into();
                    Self::append_log_locked(&mut g, loc.status_disconnected());
                    drop(g);
                    ctrl_wait.bump();
                }
            }
        });

        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if *ready.lock().unwrap() {
                let mut g = self.inner.lock().unwrap();
                if g.active_id == my_id {
                    g.state.busy = false;
                    g.state.active = true;
                    g.state.udid = udid.to_string();
                    g.state.latitude = lat;
                    g.state.longitude = lng;
                    g.state.label = label.to_string();
                    g.state.method = method_name.into();
                    g.state.error.clear();
                    g.state.message = msg_text.into();
                }
                drop(g);
                self.bump();
                return Ok(());
            }
            if *done.lock().unwrap() {
                let detail = last_lines.lock().unwrap().join(" | ");
                if !detail.is_empty() {
                    return Err(detail);
                }
                if let Some(e) = exit_err.lock().unwrap().clone() {
                    return Err(e);
                }
                return Err("pymobiledevice3 exited unexpectedly".into());
            }
            if Instant::now() >= deadline {
                self.stop_active_process();
                return Err(locale.err_connect_timeout().into());
            }
            thread::sleep(Duration::from_millis(80));
        }
    }
}

fn signal_and_wait_pid(pid: u32, done: &Arc<Mutex<bool>>, timeout: Duration) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGINT);
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if *done.lock().unwrap() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
    let kill_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < kill_deadline {
        if *done.lock().unwrap() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn search_place(query: &str, locale: Locale) -> Result<Vec<SearchResult>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    if let Some((lat, lng)) = parse_coord_query(q) {
        return Ok(vec![SearchResult {
            name: format!("{lat:.5}, {lng:.5}"),
            sub: locale.search_direct_sub().into(),
            latitude: lat,
            longitude: lng,
            direct: true,
        }]);
    }

    let endpoint = format!(
        "https://nominatim.openstreetmap.org/search?format=json&limit=6&accept-language={}&q={}",
        locale.accept_language(),
        urlencoding::encode(q)
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();

    let body = agent
        .get(&endpoint)
        .header("User-Agent", "iOS-Location-GPUI/0.2 (local desktop utility)")
        .call()
        .map_err(|e| e.to_string())?
        .into_body()
        .read_to_string()
        .map_err(|e| e.to_string())?;

    #[derive(Deserialize)]
    struct NominatimItem {
        display_name: String,
        lat: String,
        lon: String,
    }
    let raw: Vec<NominatimItem> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(raw.len());
    for it in raw {
        let (Ok(lat), Ok(lng)) = (it.lat.parse::<f64>(), it.lon.parse::<f64>()) else {
            continue;
        };
        let parts: Vec<&str> = it.display_name.split(',').map(str::trim).collect();
        let name = parts.first().copied().unwrap_or("").to_string();
        let sub = if parts.len() > 1 {
            parts[1..parts.len().min(4)].join(", ")
        } else {
            String::new()
        };
        out.push(SearchResult {
            name,
            sub,
            latitude: lat,
            longitude: lng,
            direct: false,
        });
    }
    Ok(out)
}

fn run_devicectl_set(udid: &str, lat: f64, lng: f64) -> Result<(), String> {
    let out = Command::new("/usr/bin/xcrun")
        .args([
            "devicectl",
            "device",
            "simulate",
            "location",
            "coordinate",
            "--device",
            udid,
            &format!("--latitude={lat:.7}"),
            &format!("--longitude={lng:.7}"),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

fn run_devicectl_clear(udid: &str) -> Result<(), String> {
    let out = Command::new("/usr/bin/xcrun")
        .args([
            "devicectl",
            "device",
            "simulate",
            "location",
            "clear",
            "--device",
            udid,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

fn run_pymobiledevice3_clear(
    target_id: &str,
    use_remote_pairing: bool,
    locale: Locale,
) -> Result<(), String> {
    let (bin, prefix) = find_pymobiledevice3(locale)?;
    let mode_flag = if use_remote_pairing {
        "--remotepairing"
    } else {
        "--userspace"
    };
    let mut cmd = Command::new(&bin);
    cmd.args(&prefix).args([
        "developer",
        "dvt",
        "simulate-location",
        "clear",
        mode_flag,
        "--udid",
        target_id,
    ]);
    apply_augmented_env(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let mut msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if msg.is_empty() {
            msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        return Err(if msg.is_empty() {
            format!("exit {}", out.status)
        } else {
            msg
        });
    }
    Ok(())
}

fn query_usbmux() -> Result<Vec<UsbmuxEntry>, String> {
    let (bin, prefix) = find_pymobiledevice3(Locale::En)?;
    let mut cmd = Command::new(&bin);
    cmd.args(&prefix).args(["usbmux", "list"]);
    apply_augmented_env(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let slice = text.find('[').map(|i| &text[i..]).unwrap_or(&text);
    serde_json::from_str(slice).map_err(|e| e.to_string())
}

fn query_remote_browse() -> Result<Vec<RemoteWifiEntry>, String> {
    let (bin, prefix) = find_pymobiledevice3(Locale::En)?;
    let mut cmd = Command::new(&bin);
    cmd.args(&prefix)
        .args(["remote", "browse", "--no-native", "--timeout", "2"]);
    apply_augmented_env(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let slice = text.find('{').map(|i| &text[i..]).unwrap_or(&text);
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        wifi: Vec<RemoteWifiEntry>,
    }
    let p: Payload = serde_json::from_str(slice).map_err(|e| e.to_string())?;
    Ok(p.wifi)
}

fn query_devicectl() -> Result<Vec<DevicectlEntry>, String> {
    let out = Command::new("/usr/bin/xcrun")
        .args(["devicectl", "list", "devices", "--json-output", "-"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    #[derive(Deserialize)]
    struct Root {
        result: ResultBlock,
    }
    #[derive(Deserialize)]
    struct ResultBlock {
        #[serde(default)]
        devices: Vec<DevItem>,
    }
    #[derive(Deserialize)]
    struct DevItem {
        #[serde(rename = "connectionProperties", default)]
        conn: ConnProps,
        #[serde(rename = "deviceProperties", default)]
        dev: DevProps,
        #[serde(rename = "hardwareProperties", default)]
        hw: HwProps,
    }
    #[derive(Deserialize, Default)]
    struct ConnProps {
        #[serde(rename = "transportType", default)]
        transport_type: String,
        #[serde(rename = "tunnelState", default)]
        tunnel_state: String,
    }
    #[derive(Deserialize, Default)]
    struct DevProps {
        #[serde(default)]
        name: String,
        #[serde(rename = "osVersionNumber", default)]
        os_version_number: String,
    }
    #[derive(Deserialize, Default)]
    struct HwProps {
        #[serde(default)]
        udid: String,
        #[serde(rename = "marketingName", default)]
        marketing_name: String,
        #[serde(default)]
        reality: String,
    }
    let root: Root = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let mut res = Vec::new();
    for d in root.result.devices {
        if d.hw.reality != "physical" || d.hw.udid.is_empty() {
            continue;
        }
        let connected = d.conn.tunnel_state == "connected"
            || d.conn.transport_type == "wired"
            || d.conn.transport_type == "localNetwork";
        res.push(DevicectlEntry {
            udid: d.hw.udid,
            name: d.dev.name,
            model: d.hw.marketing_name,
            os_version: d.dev.os_version_number,
            connected,
            transport: d.conn.transport_type,
        });
    }
    Ok(res)
}

fn merge_devices(
    usbmux: &[UsbmuxEntry],
    devctl: &[DevicectlEntry],
    remote_wifi: &[RemoteWifiEntry],
) -> Vec<Device> {
    let mut by_udid: HashMap<String, Device> = HashMap::new();
    let mut has_usb: HashSet<String> = HashSet::new();
    let mut has_wifi: HashSet<String> = HashSet::new();

    for u in usbmux {
        let udid = if !u.unique_device_id.is_empty() {
            u.unique_device_id.clone()
        } else {
            u.identifier.clone()
        };
        if udid.is_empty() {
            continue;
        }
        let entry = by_udid.entry(udid.clone()).or_insert_with(|| Device {
            udid: udid.clone(),
            name: u.device_name.clone(),
            model: u.product_type.clone(),
            os_version: u.product_version.clone(),
            transport: String::new(),
            can_devicectl: false,
            can_userspace: false,
            can_remote_pairing: false,
            remote_identifier: String::new(),
        });
        entry.can_userspace = true;
        if u.connection_type.eq_ignore_ascii_case("Network") {
            has_wifi.insert(udid);
        } else {
            has_usb.insert(udid);
        }
    }

    for rw in remote_wifi {
        if rw.identifier.is_empty() {
            continue;
        }
        let entry = by_udid
            .entry(rw.identifier.clone())
            .or_insert_with(|| Device {
                udid: rw.identifier.clone(),
                name: String::new(),
                model: String::new(),
                os_version: String::new(),
                transport: String::new(),
                can_devicectl: false,
                can_userspace: false,
                can_remote_pairing: false,
                remote_identifier: String::new(),
            });
        entry.can_remote_pairing = true;
        entry.remote_identifier = rw.identifier.clone();
        has_wifi.insert(rw.identifier.clone());
    }

    for dc in devctl {
        if let Some(d) = by_udid.get_mut(&dc.udid) {
            if !dc.name.is_empty() && d.name.is_empty() {
                d.name = dc.name.clone();
            }
            if !dc.model.is_empty() {
                d.model = dc.model.clone();
            }
            if !dc.os_version.is_empty() {
                d.os_version = dc.os_version.clone();
            }
            if dc.connected {
                d.can_devicectl = true;
                if dc.transport == "wired" {
                    has_usb.insert(dc.udid.clone());
                } else if dc.transport == "localNetwork" {
                    has_wifi.insert(dc.udid.clone());
                }
            }
        } else if dc.connected {
            let mut d = Device {
                udid: dc.udid.clone(),
                name: dc.name.clone(),
                model: dc.model.clone(),
                os_version: dc.os_version.clone(),
                transport: String::new(),
                can_devicectl: true,
                can_userspace: false,
                can_remote_pairing: false,
                remote_identifier: String::new(),
            };
            if dc.transport == "wired" {
                has_usb.insert(dc.udid.clone());
            } else if dc.transport == "localNetwork" {
                has_wifi.insert(dc.udid.clone());
            }
            d.can_devicectl = true;
            by_udid.insert(dc.udid.clone(), d);
        }
    }

    let mut out: Vec<Device> = by_udid
        .into_iter()
        .map(|(udid, mut d)| {
            d.transport = match (has_usb.contains(&udid), has_wifi.contains(&udid)) {
                (true, true) => "usb+wifi".into(),
                (false, true) => "wifi".into(),
                _ => "usb".into(),
            };
            if d.name.is_empty() {
                d.name = udid;
            }
            d
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn parse_pair_pin(line: &str) -> Option<String> {
    let idx = line.find("Enter this code on your device:")?;
    let rest = line[idx + "Enter this code on your device:".len()..].trim();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() == 6 {
        Some(digits)
    } else {
        None
    }
}

fn apply_augmented_env(cmd: &mut Command) {
    let home = std::env::var("HOME").unwrap_or_default();
    let base_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!(
        "{home}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:{base_path}"
    );
    cmd.env("PATH", new_path)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("PYTHONUNBUFFERED", "1");
}

fn find_pymobiledevice3(locale: Locale) -> Result<(String, Vec<String>), String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        PathBuf::from(&home).join(".local/bin/pymobiledevice3"),
        PathBuf::from("/opt/homebrew/bin/pymobiledevice3"),
        PathBuf::from("/usr/local/bin/pymobiledevice3"),
    ];
    for p in candidates {
        if p.is_file() {
            return Ok((p.to_string_lossy().to_string(), Vec::new()));
        }
    }
    let uvx_candidates = [
        PathBuf::from(&home).join(".local/bin/uvx"),
        PathBuf::from("/opt/homebrew/bin/uvx"),
    ];
    for p in uvx_candidates {
        if p.is_file() {
            return Ok((
                p.to_string_lossy().to_string(),
                vec!["pymobiledevice3".into()],
            ));
        }
    }
    Err(locale.err_pymobiledevice_missing().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pair_pin() {
        assert_eq!(
            parse_pair_pin("  Enter this code on your device: 482910"),
            Some("482910".into())
        );
        assert_eq!(parse_pair_pin("Waiting for a device..."), None);
    }

    #[test]
    fn test_merge_devices() {
        let usbmux = vec![UsbmuxEntry {
            identifier: String::new(),
            unique_device_id: "UDID-1".into(),
            device_name: "Test iPhone".into(),
            product_type: "iPhone18,2".into(),
            product_version: "27.2.1".into(),
            connection_type: "Network".into(),
        }];
        let devctl = vec![
            DevicectlEntry {
                udid: "UDID-1".into(),
                name: "Test iPhone".into(),
                model: "iPhone 17 Pro Max".into(),
                os_version: "27.2.1".into(),
                connected: false,
                transport: String::new(),
            },
            DevicectlEntry {
                udid: "UDID-2".into(),
                name: "Offline iPhone".into(),
                model: String::new(),
                os_version: String::new(),
                connected: false,
                transport: String::new(),
            },
        ];
        let devs = merge_devices(&usbmux, &devctl, &[]);
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].transport, "wifi");
        assert_eq!(devs[0].model, "iPhone 17 Pro Max");
        assert!(devs[0].can_userspace);
        assert!(!devs[0].can_devicectl);

        let remote_only = merge_devices(
            &[],
            &devctl,
            &[RemoteWifiEntry {
                identifier: "UDID-1".into(),
            }],
        );
        assert_eq!(remote_only.len(), 1);
        assert!(remote_only[0].can_remote_pairing);
        assert_eq!(remote_only[0].name, "Test iPhone");
        assert_eq!(remote_only[0].transport, "wifi");
    }
}
