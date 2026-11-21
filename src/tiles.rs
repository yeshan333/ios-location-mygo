use crate::i18n::Locale;
use crate::storage::tile_cache_dir;
use gpui::RenderImage;
use image::Frame;
use smallvec::SmallVec;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapLayer {
    GoogleMaps,
    GoogleSat,
    EsriStreets,
    AutoNavi,
    AutoNaviSat,
}

impl MapLayer {
    pub const ALL: [MapLayer; 5] = [
        MapLayer::GoogleMaps,
        MapLayer::GoogleSat,
        MapLayer::EsriStreets,
        MapLayer::AutoNavi,
        MapLayer::AutoNaviSat,
    ];

    pub fn is_gcj(self) -> bool {
        matches!(
            self,
            MapLayer::GoogleMaps | MapLayer::GoogleSat | MapLayer::AutoNavi | MapLayer::AutoNaviSat
        )
    }

    pub fn max_zoom(self) -> u8 {
        match self {
            MapLayer::GoogleMaps | MapLayer::GoogleSat => 20,
            MapLayer::EsriStreets | MapLayer::AutoNavi | MapLayer::AutoNaviSat => 18,
        }
    }

    pub fn label(self, locale: Locale) -> &'static str {
        match (self, locale) {
            (MapLayer::GoogleMaps, Locale::ZhHans) => "谷歌地图",
            (MapLayer::GoogleSat, Locale::ZhHans) => "谷歌卫星",
            (MapLayer::EsriStreets, Locale::ZhHans) => "Esri 街道",
            (MapLayer::AutoNavi, Locale::ZhHans) => "高德地图",
            (MapLayer::AutoNaviSat, Locale::ZhHans) => "高德卫星",
            (MapLayer::GoogleMaps, Locale::En) => "Google Maps",
            (MapLayer::GoogleSat, Locale::En) => "Google Sat",
            (MapLayer::EsriStreets, Locale::En) => "Esri Streets",
            (MapLayer::AutoNavi, Locale::En) => "AutoNavi",
            (MapLayer::AutoNaviSat, Locale::En) => "AutoNavi Sat",
        }
    }

    pub fn attribution(self) -> &'static str {
        match self {
            MapLayer::GoogleMaps => "© Google Maps",
            MapLayer::GoogleSat => "© Google Satellite",
            MapLayer::EsriStreets => "© Esri World Street Map",
            MapLayer::AutoNavi | MapLayer::AutoNaviSat => "© AutoNavi",
        }
    }

    pub fn has_overlay(self) -> bool {
        matches!(self, MapLayer::AutoNaviSat)
    }

    pub fn cache_prefix(self, locale: Locale, overlay: bool) -> String {
        match self {
            MapLayer::GoogleMaps => format!("gmap_{}", locale.google_hl()),
            MapLayer::GoogleSat => format!("gsat_{}", locale.google_hl()),
            MapLayer::EsriStreets => "esri".to_string(),
            MapLayer::AutoNavi => "amap".to_string(),
            MapLayer::AutoNaviSat => {
                if overlay {
                    "asat_lbl".to_string()
                } else {
                    "asat_img".to_string()
                }
            }
        }
    }

    pub fn tile_url(self, locale: Locale, z: u8, x: i32, y: i32, overlay: bool) -> String {
        let s4 = ((x.wrapping_abs() + y.wrapping_abs()) % 4) as u8;
        match self {
            MapLayer::GoogleMaps => format!(
                "https://mt{s4}.google.com/vt/lyrs=m&hl={}&x={x}&y={y}&z={z}",
                locale.google_hl()
            ),
            MapLayer::GoogleSat => format!(
                "https://mt{s4}.google.com/vt/lyrs=y&hl={}&x={x}&y={y}&z={z}",
                locale.google_hl()
            ),
            MapLayer::EsriStreets => format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{z}/{y}/{x}"
            ),
            MapLayer::AutoNavi => format!(
                "https://wprd0{}.is.autonavi.com/appmaptile?x={x}&y={y}&z={z}&lang=zh_cn&size=1&scl=1&style=7",
                s4 + 1
            ),
            MapLayer::AutoNaviSat => {
                if overlay {
                    format!(
                        "https://wprd0{}.is.autonavi.com/appmaptile?x={x}&y={y}&z={z}&lang=zh_cn&size=1&scl=1&style=8",
                        s4 + 1
                    )
                } else {
                    format!(
                        "https://webst0{}.is.autonavi.com/appmaptile?style=6&x={x}&y={y}&z={z}",
                        s4 + 1
                    )
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub layer: MapLayer,
    pub locale: Locale,
    pub overlay: bool,
    pub z: u8,
    pub x: i32,
    pub y: i32,
}

impl TileKey {
    fn disk_path(self, root: &PathBuf) -> PathBuf {
        let lang_locale = match self.layer {
            MapLayer::GoogleMaps | MapLayer::GoogleSat => self.locale,
            _ => Locale::ZhHans,
        };
        root.join(self.layer.cache_prefix(lang_locale, self.overlay))
            .join(self.z.to_string())
            .join(self.x.to_string())
            .join(format!("{}.tile", self.y))
    }

    pub fn canonical(mut self) -> Self {
        if !matches!(self.layer, MapLayer::GoogleMaps | MapLayer::GoogleSat) {
            self.locale = Locale::ZhHans;
        }
        self
    }
}

/// Decode raw PNG/JPEG/WebP bytes into a GPUI Metal-ready BGRA `RenderImage`.
pub fn decode_render_image(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let mut rgba = image::load_from_memory(bytes).ok()?.into_rgba8();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2); // RGBA -> BGRA for macOS Metal
    }
    Some(Arc::new(RenderImage::new(SmallVec::from_elem(
        Frame::new(rgba),
        1,
    ))))
}

struct SharedTileState {
    cache: HashMap<TileKey, Arc<RenderImage>>,
    order: VecDeque<TileKey>,
    inflight: HashSet<TileKey>,
    queue: VecDeque<TileKey>,
}

pub struct TileManager {
    state: Arc<(Mutex<SharedTileState>, Condvar)>,
    generation: Arc<AtomicU64>,
}

const MAX_MEM_TILES: usize = 420;
const WORKER_COUNT: usize = 6;

impl TileManager {
    pub fn new() -> Self {
        let state = Arc::new((
            Mutex::new(SharedTileState {
                cache: HashMap::with_capacity(MAX_MEM_TILES),
                order: VecDeque::with_capacity(MAX_MEM_TILES),
                inflight: HashSet::new(),
                queue: VecDeque::new(),
            }),
            Condvar::new(),
        ));
        let generation = Arc::new(AtomicU64::new(0));
        let cache_root = tile_cache_dir();

        for _ in 0..WORKER_COUNT {
            let state_cl = Arc::clone(&state);
            let gen_cl = Arc::clone(&generation);
            let root_cl = cache_root.clone();
            thread::spawn(move || {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(10)))
                    .build()
                    .into();

                loop {
                    let key = {
                        let (lock, cvar) = &*state_cl;
                        let mut guard = lock.lock().unwrap();
                        while guard.queue.is_empty() {
                            guard = cvar.wait(guard).unwrap();
                        }
                        // LIFO pops the most recently visible viewport tiles first!
                        guard.queue.pop_back().unwrap()
                    };

                    let disk_path = key.disk_path(&root_cl);
                    let maybe_bytes = if let Ok(b) = fs::read(&disk_path) {
                        Some(b)
                    } else {
                        let url =
                            key.layer
                                .tile_url(key.locale, key.z, key.x, key.y, key.overlay);
                        match agent
                            .get(&url)
                            .header(
                                "User-Agent",
                                "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15",
                            )
                            .call()
                        {
                            Ok(resp) => match resp.into_body().read_to_vec() {
                                Ok(bytes) if !bytes.is_empty() => {
                                    if let Some(parent) = disk_path.parent() {
                                        let _ = fs::create_dir_all(parent);
                                    }
                                    let _ = fs::write(&disk_path, &bytes);
                                    Some(bytes)
                                }
                                _ => None,
                            },
                            Err(_) => None,
                        }
                    };

                    let decoded = maybe_bytes.as_deref().and_then(decode_render_image);
                    let (lock, _) = &*state_cl;
                    let mut guard = lock.lock().unwrap();
                    guard.inflight.remove(&key);
                    if let Some(img) = decoded {
                        guard.cache.insert(key, img);
                        guard.order.push_back(key);
                        while guard.order.len() > MAX_MEM_TILES {
                            if let Some(old) = guard.order.pop_front() {
                                guard.cache.remove(&old);
                            }
                        }
                        gen_cl.fetch_add(1, Ordering::Release);
                    }
                }
            });
        }

        Self { state, generation }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Look up a tile in memory; if missing, enqueue it for async background fetch.
    pub fn get_or_request(&self, key: TileKey) -> Option<Arc<RenderImage>> {
        let key = key.canonical();
        let (lock, cvar) = &*self.state;
        let mut guard = lock.lock().unwrap();
        if let Some(img) = guard.cache.get(&key) {
            return Some(Arc::clone(img));
        }
        if !guard.inflight.contains(&key) {
            guard.inflight.insert(key);
            if guard.queue.len() > 160 {
                if let Some(dropped) = guard.queue.pop_front() {
                    guard.inflight.remove(&dropped);
                }
            }
            guard.queue.push_back(key);
            cvar.notify_one();
        }
        None
    }
}
