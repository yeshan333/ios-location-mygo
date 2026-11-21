use crate::coords::{
    TILE_SIZE, clamp_mercator_lat, gcj_to_wgs, lat_lng_to_world_px, wgs_to_gcj, world_px_to_lat_lng,
};
use crate::i18n::Locale;
use crate::spoofer::{Device, PairingState, SearchResult, SpooferController, SpooferState, search_place};
use crate::storage::{
    AppSettings, Favorite, add_favorite_to, app_data_dir, delete_favorite_from, list_favorites_from,
    save_settings_to,
};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::Theme;
use crate::tiles::{MapLayer, TileKey, TileManager, decode_render_image};
use gpui::{
    App, AsyncApp, Bounds, BoxShadow, ClipboardItem, Context, Corners, CursorStyle, Entity,
    FocusHandle, Focusable, FontWeight, Hsla, Menu, MenuItem, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, RenderImage, ScrollDelta,
    ScrollWheelEvent, SharedString, Subscription, SystemMenuType, WeakEntity, Window, actions,
    canvas, div, fill, img, point, prelude::*, px, rgba, size,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

actions!(
    app_actions,
    [
        QuitApp,
        TogglePanel,
        FocusSearch,
        SwitchLanguage,
        ToggleTheme,
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlacesTab {
    Cities,
    Favorites,
}

struct PresetCity {
    idx: usize,
    region: &'static str,
    lat: f64,
    lng: f64,
}

const PRESETS: [PresetCity; 6] = [
    PresetCity { idx: 0, region: "US", lat: 34.0536923, lng: -118.2427676 },
    PresetCity { idx: 1, region: "US", lat: 37.7749295, lng: -122.4194155 },
    PresetCity { idx: 2, region: "JP", lat: 35.6585805, lng: 139.7454329 },
    PresetCity { idx: 3, region: "SG", lat: 1.2833754, lng: 103.8607264 },
    PresetCity { idx: 4, region: "UK", lat: 51.5007292, lng: -0.1246254 },
    PresetCity { idx: 5, region: "CN", lat: 39.90872, lng: 116.39748 },
];

pub fn install_app_menus(locale: Locale, cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: locale.app_title().into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(locale.menu_quit(), QuitApp),
            ],
        },
        Menu {
            name: locale.menu_file().into(),
            items: vec![MenuItem::action(locale.menu_focus_search(), FocusSearch)],
        },
        Menu {
            name: locale.menu_edit().into(),
            items: vec![
                MenuItem::action("Cut", crate::text_input::Cut),
                MenuItem::action("Copy", crate::text_input::Copy),
                MenuItem::action("Paste", crate::text_input::Paste),
                MenuItem::action("Select All", crate::text_input::SelectAll),
            ],
        },
        Menu {
            name: locale.menu_view().into(),
            items: vec![
                MenuItem::action(locale.menu_toggle_panel(), TogglePanel),
                MenuItem::action(locale.menu_toggle_theme(), ToggleTheme),
                MenuItem::separator(),
                MenuItem::action(locale.menu_switch_lang(), SwitchLanguage),
            ],
        },
    ]);
}

pub struct AppView {
    focus_handle: FocusHandle,
    data_dir: PathBuf,
    locale: Locale,
    dark_mode: bool,
    app_icon: Option<Arc<RenderImage>>,

    // Map state (always stored in true WGS-84)
    wgs_lat: f64,
    wgs_lng: f64,
    zoom: f64,
    layer: MapLayer,
    tiles: Arc<TileManager>,
    last_tile_gen: u64,
    map_bounds: Bounds<Pixels>,

    // Drag gesture state
    dragging: bool,
    drag_moved: bool,
    drag_start_pos: Point<Pixels>,
    drag_start_world: (f64, f64),

    // Spoofer backend & mirrored snapshots
    spoofer: SpooferController,
    last_spoofer_ver: u64,
    state: SpooferState,
    devices: Vec<Device>,
    devices_scanned: bool,
    selected_udid: String,
    pairing: PairingState,
    pair_modal_open: bool,

    // UI toggles & inputs
    panel_collapsed: bool,
    device_dropdown_open: bool,
    places_tab: PlacesTab,
    log_drawer_open: bool,
    fav_form_open: bool,
    favorites: Vec<Favorite>,

    search_input: Entity<TextInput>,
    fav_input: Entity<TextInput>,
    search_open: bool,
    search_busy: bool,
    search_results: Vec<SearchResult>,
    search_error: Option<String>,

    toast: Option<(String, Instant)>,
    copied_until: Option<Instant>,
    _subs: Vec<Subscription>,
}

impl Focusable for AppView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AppView {
    pub fn new(
        locale: Locale,
        dark_mode: bool,
        spoofer: SpooferController,
        cx: &mut Context<Self>,
    ) -> Self {
        let data_dir = app_data_dir();
        let favorites = list_favorites_from(&data_dir);
        let theme = if dark_mode { Theme::dark() } else { Theme::light() };

        let icon_bytes = include_bytes!("../assets/app-icon.png");
        let app_icon = decode_render_image(icon_bytes);

        let search_input = cx.new(|cx| {
            TextInput::new(
                cx,
                locale.search_placeholder(),
                theme.text,
                theme.muted,
                theme.accent,
            )
        });
        let fav_input = cx.new(|cx| {
            TextInput::new(
                cx,
                locale.fav_placeholder(),
                theme.text,
                theme.muted,
                theme.accent,
            )
        });

        let sub1 = cx.subscribe(&search_input, |this, _, ev: &TextInputEvent, cx| match ev {
            TextInputEvent::Submit(q) => this.run_search(q.clone(), cx),
            TextInputEvent::Cancel => {
                this.search_open = false;
                cx.notify();
            }
            TextInputEvent::Changed(txt) => {
                if txt.trim().is_empty() {
                    this.search_open = false;
                    this.search_results.clear();
                    this.search_error = None;
                }
                cx.notify();
            }
        });

        let sub2 = cx.subscribe(&fav_input, |this, _, ev: &TextInputEvent, cx| match ev {
            TextInputEvent::Submit(name) => this.submit_favorite(name.clone(), cx),
            TextInputEvent::Cancel => {
                this.fav_form_open = false;
                cx.notify();
            }
            TextInputEvent::Changed(_) => {}
        });

        let tiles = Arc::new(TileManager::new());

        // Trigger initial 3-way device scan on boot.
        spoofer.scan_devices_async();

        // Background poller (60ms) syncing tile arrivals, backend state updates, and toast expiry.
        cx.spawn(async move |this: WeakEntity<AppView>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(60))
                    .await;
                let ok = this.update(cx, |view, cx| {
                    let mut dirty = false;
                    let tgen = view.tiles.generation();
                    if tgen != view.last_tile_gen {
                        view.last_tile_gen = tgen;
                        dirty = true;
                    }
                    let sver = view.spoofer.version();
                    if sver != view.last_spoofer_ver {
                        view.last_spoofer_ver = sver;
                        let prev_active = view.state.active;
                        let prev_busy = view.state.busy;
                        let prev_pair_status = view.pairing.status.clone();
                        let (st, devs, pair) = view.spoofer.snapshot();

                        // Emit helpful toasts on state transitions
                        if prev_busy && !st.busy {
                            if st.active && !prev_active {
                                view.show_toast(view.locale.toast_locked());
                            } else if !st.error.is_empty() && prev_active {
                                view.show_toast(view.locale.toast_clear_failed());
                            } else if !st.error.is_empty() && !st.active {
                                view.show_toast(view.locale.toast_set_failed());
                            } else if !st.active && st.error.is_empty() && prev_active {
                                view.show_toast(view.locale.toast_cleared());
                            }
                        }
                        if pair.status == "paired" && prev_pair_status != "paired" {
                            let name = if pair.device_name.is_empty() {
                                "iPhone"
                            } else {
                                &pair.device_name
                            };
                            view.show_toast(format!("{}{name}", view.locale.toast_paired_prefix()));
                            view.pair_modal_open = false;
                        }

                        if !devs.is_empty() || !st.logs.is_empty() {
                            view.devices_scanned = true;
                        }
                        if !devs.is_empty()
                            && !devs.iter().any(|d| d.udid == view.selected_udid)
                        {
                            view.selected_udid = devs[0].udid.clone();
                        } else if devs.is_empty() {
                            view.selected_udid.clear();
                        }

                        view.state = st;
                        view.devices = devs;
                        view.pairing = pair;
                        dirty = true;
                    }

                    let now = Instant::now();
                    if let Some((_, until)) = view.toast {
                        if now >= until {
                            view.toast = None;
                            dirty = true;
                        }
                    }
                    if let Some(until) = view.copied_until {
                        if now >= until {
                            view.copied_until = None;
                            dirty = true;
                        }
                    }

                    if dirty {
                        cx.notify();
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            focus_handle: cx.focus_handle(),
            data_dir,
            locale,
            dark_mode,
            app_icon,
            wgs_lat: 34.0536923,
            wgs_lng: -118.2427676,
            zoom: 13.0,
            layer: MapLayer::GoogleMaps,
            tiles,
            last_tile_gen: 0,
            map_bounds: Bounds::new(point(px(0.), px(0.)), size(px(1180.), px(780.))),
            dragging: false,
            drag_moved: false,
            drag_start_pos: point(px(0.), px(0.)),
            drag_start_world: (0.0, 0.0),
            spoofer,
            last_spoofer_ver: 0,
            state: SpooferState::default(),
            devices: Vec::new(),
            devices_scanned: false,
            selected_udid: String::new(),
            pairing: PairingState {
                status: "idle".into(),
                ..Default::default()
            },
            pair_modal_open: false,
            panel_collapsed: false,
            device_dropdown_open: false,
            places_tab: PlacesTab::Cities,
            log_drawer_open: false,
            fav_form_open: false,
            favorites,
            search_input,
            fav_input,
            search_open: false,
            search_busy: false,
            search_results: Vec::new(),
            search_error: None,
            toast: None,
            copied_until: None,
            _subs: vec![sub1, sub2],
        }
    }

    fn theme(&self) -> Theme {
        if self.dark_mode {
            Theme::dark()
        } else {
            Theme::light()
        }
    }

    fn persist_settings(&self) {
        let _ = save_settings_to(
            &self.data_dir,
            &AppSettings {
                locale: Some(self.locale),
                dark_mode: Some(self.dark_mode),
            },
        );
    }

    fn sync_input_styles(&self, cx: &mut Context<Self>) {
        let th = self.theme();
        let loc = self.locale;
        self.search_input.update(cx, |inp, cx| {
            inp.set_placeholder(loc.search_placeholder(), cx);
            inp.set_colors(th.text, th.muted, th.accent, cx);
        });
        self.fav_input.update(cx, |inp, cx| {
            inp.set_placeholder(loc.fav_placeholder(), cx);
            inp.set_colors(th.text, th.muted, th.accent, cx);
        });
    }

    fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now() + Duration::from_millis(2400)));
    }

    fn wgs_to_disp(&self, lat: f64, lng: f64) -> (f64, f64) {
        if self.layer.is_gcj() {
            wgs_to_gcj(lat, lng)
        } else {
            (lat, lng)
        }
    }

    fn disp_to_wgs(&self, lat: f64, lng: f64) -> (f64, f64) {
        if self.layer.is_gcj() {
            gcj_to_wgs(lat, lng)
        } else {
            (lat, lng)
        }
    }

    fn jump_to_wgs(&mut self, lat: f64, lng: f64, min_zoom: f64, cx: &mut Context<Self>) {
        self.wgs_lat = clamp_mercator_lat(lat);
        self.wgs_lng = lng;
        self.zoom = self.zoom.max(min_zoom).min(self.layer.max_zoom() as f64);
        cx.notify();
    }

    fn copy_coords(&mut self, cx: &mut Context<Self>) {
        let txt = format!("{:.6}, {:.6}", self.wgs_lat, self.wgs_lng);
        cx.write_to_clipboard(ClipboardItem::new_string(txt.clone()));
        self.copied_until = Some(Instant::now() + Duration::from_millis(1500));
        self.show_toast(format!("{}{txt}", self.locale.copied_toast()));
        cx.notify();
    }

    fn toggle_locale(&mut self, cx: &mut Context<Self>) {
        self.locale = self.locale.toggle();
        self.spoofer.set_locale(self.locale);
        self.persist_settings();
        self.sync_input_styles(cx);
        install_app_menus(self.locale, cx);
        cx.notify();
    }

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.dark_mode = !self.dark_mode;
        self.persist_settings();
        self.sync_input_styles(cx);
        cx.notify();
    }

    fn lock_location(&mut self, lat: f64, lng: f64, label: String, cx: &mut Context<Self>) {
        if self.selected_udid.is_empty() {
            self.show_toast(self.locale.toast_need_device());
            cx.notify();
            return;
        }
        self.spoofer
            .set_location_async(self.selected_udid.clone(), lat, lng, label);
        cx.notify();
    }

    fn clear_location(&mut self, cx: &mut Context<Self>) {
        self.spoofer.clear_location_async(self.selected_udid.clone());
        cx.notify();
    }

    fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        let q = query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.search_open = true;
        self.search_busy = true;
        self.search_error = None;
        self.search_results.clear();
        let locale = self.locale;
        cx.notify();

        cx.spawn(async move |this: WeakEntity<AppView>, cx: &mut AsyncApp| {
            let res = cx
                .background_executor()
                .spawn(async move { search_place(&q, locale) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.search_busy = false;
                match res {
                    Ok(items) => {
                        if items.len() == 1 && items[0].direct {
                            view.jump_to_wgs(items[0].latitude, items[0].longitude, 15.0, cx);
                            view.search_open = false;
                            view.show_toast(view.locale.jumped_coord());
                        } else {
                            view.search_results = items;
                        }
                    }
                    Err(e) => {
                        view.search_error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn submit_favorite(&mut self, name: String, cx: &mut Context<Self>) {
        if let Ok(list) = add_favorite_to(&self.data_dir, &name, self.wgs_lat, self.wgs_lng) {
            self.favorites = list;
            self.fav_form_open = false;
            self.places_tab = PlacesTab::Favorites;
            self.fav_input.update(cx, |inp, cx| inp.clear(cx));
            self.show_toast(self.locale.fav_added());
            cx.notify();
        }
    }

    fn delete_favorite(&mut self, id: String, cx: &mut Context<Self>) {
        if let Ok(list) = delete_favorite_from(&self.data_dir, &id) {
            self.favorites = list;
            self.show_toast(self.locale.fav_deleted());
            cx.notify();
        }
    }

    fn open_pairing(&mut self, cx: &mut Context<Self>) {
        self.pair_modal_open = true;
        self.spoofer.start_wifi_pairing("MacBook-Location");
        cx.notify();
    }

    fn close_pairing(&mut self, cx: &mut Context<Self>) {
        self.pair_modal_open = false;
        self.spoofer.stop_wifi_pairing();
        cx.notify();
    }

    // --- Map interaction handlers ---
    fn on_map_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        self.device_dropdown_open = false;
        self.search_open = false;
        self.dragging = true;
        self.drag_moved = false;
        self.drag_start_pos = ev.position;
        let (dlat, dlng) = self.wgs_to_disp(self.wgs_lat, self.wgs_lng);
        self.drag_start_world = lat_lng_to_world_px(dlat, dlng, self.zoom);
        cx.notify();
    }

    fn on_map_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dragging {
            return;
        }
        let dx: f64 = (ev.position.x - self.drag_start_pos.x).into();
        let dy: f64 = (ev.position.y - self.drag_start_pos.y).into();
        if dx.abs() > 3.0 || dy.abs() > 3.0 {
            self.drag_moved = true;
        }
        let new_wx = self.drag_start_world.0 - dx;
        let new_wy = self.drag_start_world.1 - dy;
        let (dlat, dlng) = world_px_to_lat_lng(new_wx, new_wy, self.zoom);
        let (wlat, wlng) = self.disp_to_wgs(dlat, dlng);
        self.wgs_lat = wlat;
        self.wgs_lng = wlng;
        cx.notify();
    }

    fn on_map_mouse_up(&mut self, ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        if !self.drag_moved {
            // Single click on map -> center on clicked point!
            let b = self.map_bounds;
            let cx_px: f64 = (b.origin.x + b.size.width * 0.5).into();
            let cy_px: f64 = (b.origin.y + b.size.height * 0.5).into();
            let px_x: f64 = ev.position.x.into();
            let px_y: f64 = ev.position.y.into();
            let (dlat, dlng) = self.wgs_to_disp(self.wgs_lat, self.wgs_lng);
            let (wx, wy) = lat_lng_to_world_px(dlat, dlng, self.zoom);
            let target_wx = wx + (px_x - cx_px);
            let target_wy = wy + (px_y - cy_px);
            let (ndlat, ndlng) = world_px_to_lat_lng(target_wx, target_wy, self.zoom);
            let (wlat, wlng) = self.disp_to_wgs(ndlat, ndlng);
            self.wgs_lat = wlat;
            self.wgs_lng = wlng;
        }
        cx.notify();
    }

    fn on_map_scroll(
        &mut self,
        ev: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta_y: f64 = match ev.delta {
            ScrollDelta::Pixels(p) => {
                let py: f64 = p.y.into();
                py / 220.0
            }
            ScrollDelta::Lines(p) => (p.y as f64) * 0.35,
        };
        if delta_y.abs() < 1e-4 {
            return;
        }
        let max_z = self.layer.max_zoom() as f64;
        let old_zoom = self.zoom;
        let new_zoom = (old_zoom + delta_y).clamp(2.5, max_z);
        if (new_zoom - old_zoom).abs() < 1e-4 {
            return;
        }

        // Keep geographic point under mouse cursor fixed while zooming!
        let b = self.map_bounds;
        let center_x: f64 = (b.origin.x + b.size.width * 0.5).into();
        let center_y: f64 = (b.origin.y + b.size.height * 0.5).into();
        let mx: f64 = ev.position.x.into();
        let my: f64 = ev.position.y.into();
        let off_x = mx - center_x;
        let off_y = my - center_y;

        let (dlat, dlng) = self.wgs_to_disp(self.wgs_lat, self.wgs_lng);
        let (old_wx, old_wy) = lat_lng_to_world_px(dlat, dlng, old_zoom);
        let (cursor_dlat, cursor_dlng) = world_px_to_lat_lng(old_wx + off_x, old_wy + off_y, old_zoom);
        let (new_cursor_wx, new_cursor_wy) = lat_lng_to_world_px(cursor_dlat, cursor_dlng, new_zoom);
        let (new_dlat, new_dlng) =
            world_px_to_lat_lng(new_cursor_wx - off_x, new_cursor_wy - off_y, new_zoom);
        let (wlat, wlng) = self.disp_to_wgs(new_dlat, new_dlng);

        self.zoom = new_zoom;
        self.wgs_lat = wlat;
        self.wgs_lng = wlng;
        cx.notify();
    }

    fn render_map_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tiles = Arc::clone(&self.tiles);
        let layer = self.layer;
        let locale = self.locale;
        let zoom = self.zoom;
        let (dlat, dlng) = self.wgs_to_disp(self.wgs_lat, self.wgs_lng);
        let locked_disp = if self.state.active {
            Some(self.wgs_to_disp(self.state.latitude, self.state.longitude))
        } else {
            None
        };
        let moving = self.dragging && self.drag_moved;
        let bg = self.theme().canvas_bg;
        let entity = cx.entity();

        div()
            .id("map-canvas")
            .absolute()
            .inset_0()
            .bg(bg)
            .cursor(if self.dragging {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::OpenHand
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_map_mouse_down))
            .on_mouse_move(cx.listener(Self::on_map_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_map_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_map_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_map_scroll))
            .child(
                canvas(
                    move |bounds, _window, cx| {
                        entity.update(cx, |v, _| v.map_bounds = bounds);
                        bounds
                    },
                    move |bounds, _, window, _cx| {
                        let w: f64 = bounds.size.width.into();
                        let h: f64 = bounds.size.height.into();
                        if w <= 1.0 || h <= 1.0 {
                            return;
                        }
                        let origin_x: f64 = bounds.origin.x.into();
                        let origin_y: f64 = bounds.origin.y.into();

                        let z = (zoom.floor() as i32).clamp(2, layer.max_zoom() as i32) as u8;
                        let scale = 2.0_f64.powf(zoom - z as f64);
                        let tile_px = TILE_SIZE * scale;
                        let n_tiles = 1_i32 << z;

                        let (center_wx, center_wy) = lat_lng_to_world_px(dlat, dlng, zoom);
                        let top_left_wx = center_wx - w * 0.5;
                        let top_left_wy = center_wy - h * 0.5;

                        let min_tx = (top_left_wx / tile_px).floor() as i32 - 1;
                        let max_tx = ((top_left_wx + w) / tile_px).ceil() as i32;
                        let min_ty = ((top_left_wy / tile_px).floor() as i32).max(0);
                        let max_ty = (((top_left_wy + h) / tile_px).ceil() as i32).min(n_tiles - 1);

                        let tile_sz = size(px(tile_px as f32 + 0.6), px(tile_px as f32 + 0.6));

                        for ty in min_ty..=max_ty {
                            for tx in min_tx..=max_tx {
                                let wrapped_tx = tx.rem_euclid(n_tiles);
                                let sx = origin_x + (tx as f64) * tile_px - top_left_wx;
                                let sy = origin_y + (ty as f64) * tile_px - top_left_wy;
                                let t_bounds =
                                    Bounds::new(point(px(sx as f32), px(sy as f32)), tile_sz);

                                let key = TileKey {
                                    layer,
                                    locale,
                                    overlay: false,
                                    z,
                                    x: wrapped_tx,
                                    y: ty,
                                };
                                if let Some(img_data) = tiles.get_or_request(key) {
                                    let _ = window.paint_image(
                                        t_bounds,
                                        Corners::default(),
                                        img_data,
                                        0,
                                        false,
                                    );
                                } else {
                                    // Subtle grid placeholder while tile streams in
                                    window.paint_quad(fill(t_bounds, rgba(0x8899aa14)));
                                }

                                if layer.has_overlay() {
                                    let ov_key = TileKey {
                                        overlay: true,
                                        ..key
                                    };
                                    if let Some(ov_img) = tiles.get_or_request(ov_key) {
                                        let _ = window.paint_image(
                                            t_bounds,
                                            Corners::default(),
                                            ov_img,
                                            0,
                                            false,
                                        );
                                    }
                                }
                            }
                        }

                        // Paint Locked Location Beacon on map
                        if let Some((ldlat, ldlng)) = locked_disp {
                            let (lwx, lwy) = lat_lng_to_world_px(ldlat, ldlng, zoom);
                            let lx = origin_x + (lwx - top_left_wx);
                            let ly = origin_y + (lwy - top_left_wy);
                            if lx >= origin_x - 30.0
                                && lx <= origin_x + w + 30.0
                                && ly >= origin_y - 30.0
                                && ly <= origin_y + h + 30.0
                            {
                                // Soft blue halo
                                let halo = Bounds::new(
                                    point(px((lx - 18.0) as f32), px((ly - 18.0) as f32)),
                                    size(px(36.0), px(36.0)),
                                );
                                window.paint_quad(gpui::quad(
                                    halo,
                                    Corners::all(px(18.0)),
                                    rgba(0x007aff38),
                                    gpui::Edges::all(px(0.0)),
                                    rgba(0x00000000),
                                    gpui::BorderStyle::Solid,
                                ));
                                // White ring + blue core
                                let core = Bounds::new(
                                    point(px((lx - 8.0) as f32), px((ly - 8.0) as f32)),
                                    size(px(16.0), px(16.0)),
                                );
                                window.paint_quad(gpui::quad(
                                    core,
                                    Corners::all(px(8.0)),
                                    rgba(0x007affff),
                                    gpui::Edges::all(px(2.5)),
                                    rgba(0xffffffff),
                                    gpui::BorderStyle::Solid,
                                ));
                            }
                        }

                        // Paint Precision Center Reticle Crosshair
                        let cx_f = (origin_x + w * 0.5) as f32;
                        let cy_f = (origin_y + h * 0.5) as f32;
                        let r = if moving { 12.0_f32 } else { 10.0_f32 };
                        let blue = rgba(0x007affff);
                        let white_halo = rgba(0xffffffcc);

                        // Outer ring (white halo + blue stroke)
                        let ring_bounds = Bounds::new(
                            point(px(cx_f - r), px(cy_f - r)),
                            size(px(r * 2.0), px(r * 2.0)),
                        );
                        window.paint_quad(gpui::quad(
                            ring_bounds,
                            Corners::all(px(r)),
                            rgba(0x00000000),
                            gpui::Edges::all(px(3.2)),
                            white_halo,
                            gpui::BorderStyle::Solid,
                        ));
                        window.paint_quad(gpui::quad(
                            ring_bounds,
                            Corners::all(px(r)),
                            rgba(0x00000000),
                            gpui::Edges::all(px(1.8)),
                            blue,
                            gpui::BorderStyle::Solid,
                        ));

                        // 4 cardinal crosshair ticks
                        let tick_len = 8.0_f32;
                        let gap = r + 1.0;
                        let mut pb = PathBuilder::stroke(px(2.0));
                        pb.move_to(point(px(cx_f), px(cy_f - gap - tick_len)));
                        pb.line_to(point(px(cx_f), px(cy_f - gap)));
                        pb.move_to(point(px(cx_f), px(cy_f + gap)));
                        pb.line_to(point(px(cx_f), px(cy_f + gap + tick_len)));
                        pb.move_to(point(px(cx_f - gap - tick_len), px(cy_f)));
                        pb.line_to(point(px(cx_f - gap), px(cy_f)));
                        pb.move_to(point(px(cx_f + gap), px(cy_f)));
                        pb.line_to(point(px(cx_f + gap + tick_len), px(cy_f)));
                        if let Ok(path) = pb.build() {
                            window.paint_path(path, blue);
                        }

                        // Crisp center dot
                        let dot_bounds = Bounds::new(
                            point(px(cx_f - 3.0), px(cy_f - 3.0)),
                            size(px(6.0), px(6.0)),
                        );
                        window.paint_quad(gpui::quad(
                            dot_bounds,
                            Corners::all(px(3.0)),
                            blue,
                            gpui::Edges::all(px(1.2)),
                            rgba(0xffffffff),
                            gpui::BorderStyle::Solid,
                        ));
                    },
                )
                .size_full(),
            )
    }

    fn panel_shadow(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: if self.dark_mode {
                rgba(0x00000088).into()
            } else {
                rgba(0x0f172a28).into()
            },
            offset: point(px(0.), px(14.)),
            blur_radius: px(36.),
            spread_radius: px(0.),
        }]
    }

    fn pill_shadow(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: if self.dark_mode {
                rgba(0x00000070).into()
            } else {
                rgba(0x0f172a20).into()
            },
            offset: point(px(0.), px(6.)),
            blur_radius: px(20.),
            spread_radius: px(0.),
        }]
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let th = self.theme();
        let loc = self.locale;

        let selected_dev = self
            .devices
            .iter()
            .find(|d| d.udid == self.selected_udid);
        let online = selected_dev.is_some();
        let transport_label: SharedString = match selected_dev.map(|d| d.transport.as_str()) {
            Some("usb+wifi") => "USB + Wi-Fi".into(),
            Some("wifi") => "Wi-Fi".into(),
            Some(_) => "USB".into(),
            None => loc.offline().into(),
        };

        let device_btn_label: SharedString = if let Some(d) = selected_dev {
            let tag = match d.transport.as_str() {
                "wifi" => "Wi-Fi",
                "usb+wifi" => "USB+Wi-Fi",
                _ => "USB",
            };
            let ver = if d.os_version.is_empty() { "?" } else { &d.os_version };
            format!("{} (iOS {ver} · {tag})", d.name).into()
        } else if !self.devices_scanned {
            loc.scanning_devices().into()
        } else {
            loc.no_device().into()
        };

        let (status_dot_color, status_title): (Hsla, SharedString) = if self.state.busy {
            (th.amber, loc.status_syncing().into())
        } else if self.state.active {
            let title = if self.state.label.is_empty() {
                loc.status_locked().into()
            } else {
                format!("{}{}", loc.status_locked_prefix(), self.state.label).into()
            };
            (th.green, title)
        } else if !self.state.error.is_empty() {
            (th.red, loc.status_idle().into())
        } else {
            (th.muted, loc.status_idle().into())
        };

        let method_badge: Option<&'static str> = if self.state.active {
            match self.state.method.as_str() {
                "devicectl" => Some(loc.method_xcode()),
                "remotepairing" => Some(loc.method_remote()),
                "" => None,
                _ => Some(loc.method_wifi()),
            }
        } else {
            None
        };

        let status_msg: SharedString = if self.state.message.is_empty() {
            loc.status_hint().into()
        } else {
            self.state.message.clone().into()
        };

        let search_text = self.search_input.read(cx).text();
        let cities_names = loc.cities();

        div()
            .id("inspector-panel")
            .absolute()
            .top(px(14.))
            .left(px(14.))
            .bottom(px(14.))
            .w(px(326.))
            .flex()
            .flex_col()
            .rounded(px(18.))
            .bg(th.panel_bg)
            .border_1()
            .border_color(th.panel_border)
            .shadow(self.panel_shadow())
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            // 1. Titlebar row (space for macOS traffic lights on left)
            .child(
                div()
                    .h(px(34.))
                    .pl(px(82.))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(4.))
                    .child(
                        div()
                            .id("btn-lang")
                            .h(px(24.))
                            .px(px(8.))
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(th.text_secondary)
                            .cursor_pointer()
                            .hover(|s| s.bg(th.surface_hover).text_color(th.text))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_locale(cx)))
                            .child(loc.lang_btn()),
                    )
                    .child(
                        div()
                            .id("btn-theme")
                            .size(px(26.))
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(13.))
                            .text_color(th.text_secondary)
                            .cursor_pointer()
                            .hover(|s| s.bg(th.surface_hover).text_color(th.text))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx)))
                            .child(if self.dark_mode { "☀" } else { "☾" }),
                    )
                    .child(
                        div()
                            .id("btn-collapse")
                            .size(px(26.))
                            .rounded(px(7.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(13.))
                            .text_color(th.text_secondary)
                            .cursor_pointer()
                            .hover(|s| s.bg(th.surface_hover).text_color(th.text))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.panel_collapsed = true;
                                cx.notify();
                            }))
                            .child("◧"),
                    ),
            )
            // 2. Brand header
            .child(
                div()
                    .px(px(16.))
                    .pt(px(2.))
                    .pb(px(12.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .when_some(self.app_icon.clone(), |el, icon| {
                                el.child(
                                    img(icon)
                                        .size(px(28.))
                                        .rounded(px(7.)),
                                )
                            })
                            .child(
                                div()
                                    .text_size(px(15.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(th.text)
                                    .child("iOS Location"),
                            ),
                    )
                    .child(
                        div()
                            .h(px(24.))
                            .px(px(10.))
                            .rounded(px(999.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .bg(if online { th.green_soft } else { th.surface })
                            .border_1()
                            .border_color(if online { th.green.opacity(0.3) } else { th.divider })
                            .child(
                                div()
                                    .size(px(6.))
                                    .rounded(px(999.))
                                    .bg(if online { th.green } else { th.muted }),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if online { th.green } else { th.muted })
                                    .child(transport_label),
                            ),
                    ),
            )
            // 3. Scrollable body
            .child(
                div()
                    .id("panel-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(14.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(11.))
                    // 3a. Device selector row
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .id("btn-device-select")
                                            .flex_1()
                                            .min_w_0()
                                            .h(px(34.))
                                            .px(px(10.))
                                            .rounded(px(10.))
                                            .bg(th.input_bg)
                                            .border_1()
                                            .border_color(th.divider)
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap(px(6.))
                                            .cursor_pointer()
                                            .hover(|s| s.border_color(th.accent_border))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.device_dropdown_open =
                                                    !this.device_dropdown_open;
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .text_size(px(12.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(if online {
                                                        th.text
                                                    } else {
                                                        th.muted
                                                    })
                                                    .child(device_btn_label),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(th.muted)
                                                    .child("⌄"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("btn-pair")
                                            .size(px(34.))
                                            .rounded(px(10.))
                                            .bg(th.input_bg)
                                            .border_1()
                                            .border_color(th.divider)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(13.))
                                            .text_color(th.text_secondary)
                                            .cursor_pointer()
                                            .hover(|s| s.bg(th.surface_hover).text_color(th.accent))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.open_pairing(cx);
                                            }))
                                            .child("⌁"),
                                    )
                                    .child(
                                        div()
                                            .id("btn-refresh")
                                            .size(px(34.))
                                            .rounded(px(10.))
                                            .bg(th.input_bg)
                                            .border_1()
                                            .border_color(th.divider)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(14.))
                                            .text_color(th.text_secondary)
                                            .cursor_pointer()
                                            .hover(|s| s.bg(th.surface_hover).text_color(th.text))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.spoofer.scan_devices_async();
                                                cx.notify();
                                            }))
                                            .child("↻"),
                                    ),
                            )
                            // Device dropdown menu when open
                            .when(self.device_dropdown_open, |el| {
                                el.child(
                                    div()
                                        .w_full()
                                        .p(px(4.))
                                        .rounded(px(11.))
                                        .bg(th.panel_solid)
                                        .border_1()
                                        .border_color(th.panel_border)
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .when(self.devices.is_empty(), |menu| {
                                            menu.child(
                                                div()
                                                    .px(px(10.))
                                                    .py(px(8.))
                                                    .text_size(px(12.))
                                                    .text_color(th.muted)
                                                    .child(loc.no_device()),
                                            )
                                        })
                                        .children(self.devices.iter().enumerate().map(|(i, d)| {
                                            let udid = d.udid.clone();
                                            let active = udid == self.selected_udid;
                                            let tag = match d.transport.as_str() {
                                                "wifi" => "Wi-Fi",
                                                "usb+wifi" => "USB+Wi-Fi",
                                                _ => "USB",
                                            };
                                            let label = format!(
                                                "{} (iOS {} · {})",
                                                d.name,
                                                if d.os_version.is_empty() { "?" } else { &d.os_version },
                                                tag
                                            );
                                            div()
                                                .id(("dev-opt", i))
                                                .px(px(10.))
                                                .py(px(7.))
                                                .rounded(px(8.))
                                                .bg(if active {
                                                    th.accent_soft
                                                } else {
                                                    rgba(0x00000000).into()
                                                })
                                                .text_size(px(12.))
                                                .text_color(if active { th.accent } else { th.text })
                                                .cursor_pointer()
                                                .hover(|s| s.bg(th.surface_hover))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.selected_udid = udid.clone();
                                                    this.device_dropdown_open = false;
                                                    cx.notify();
                                                }))
                                                .child(label)
                                        })),
                                )
                            }),
                    )
                    // 3b. Spotlight search bar
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .h(px(34.))
                                    .px(px(10.))
                                    .rounded(px(10.))
                                    .bg(th.input_bg)
                                    .border_1()
                                    .border_color(th.divider)
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .child(
                                        div()
                                            .id("btn-search-go")
                                            .text_size(px(12.5))
                                            .text_color(th.muted)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let q = this.search_input.read(cx).text();
                                                this.run_search(q, cx);
                                            }))
                                            .child("⌕"),
                                    )
                                    .child(self.search_input.clone())
                                    .when(!search_text.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .id("btn-search-clear")
                                                .size(px(18.))
                                                .rounded(px(999.))
                                                .bg(th.surface_hover)
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_size(px(10.))
                                                .text_color(th.muted)
                                                .cursor_pointer()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.search_input.update(cx, |i, cx| i.clear(cx));
                                                    this.search_open = false;
                                                    cx.notify();
                                                }))
                                                .child("×"),
                                        )
                                    })
                                    .when(search_text.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .px(px(5.))
                                                .py(px(2.))
                                                .rounded(px(5.))
                                                .bg(th.surface)
                                                .border_1()
                                                .border_color(th.divider)
                                                .text_size(px(10.))
                                                .text_color(th.muted)
                                                .child("⌘K"),
                                        )
                                    }),
                            )
                            // Search results dropdown
                            .when(self.search_open, |el| {
                                el.child(
                                    div()
                                        .w_full()
                                        .p(px(4.))
                                        .rounded(px(11.))
                                        .bg(th.panel_solid)
                                        .border_1()
                                        .border_color(th.panel_border)
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .when(self.search_busy, |box_el| {
                                            box_el.child(
                                                div()
                                                    .px(px(10.))
                                                    .py(px(8.))
                                                    .text_size(px(12.))
                                                    .text_color(th.muted)
                                                    .child(loc.searching()),
                                            )
                                        })
                                        .when_some(self.search_error.clone(), |box_el, err| {
                                            box_el.child(
                                                div()
                                                    .px(px(10.))
                                                    .py(px(8.))
                                                    .text_size(px(11.5))
                                                    .text_color(th.red)
                                                    .child(format!("{}{err}", loc.search_failed())),
                                            )
                                        })
                                        .when(
                                            !self.search_busy
                                                && self.search_error.is_none()
                                                && self.search_results.is_empty(),
                                            |box_el| {
                                                box_el.child(
                                                    div()
                                                        .px(px(10.))
                                                        .py(px(8.))
                                                        .text_size(px(12.))
                                                        .text_color(th.muted)
                                                        .child(loc.search_empty()),
                                                )
                                            },
                                        )
                                        .children(self.search_results.iter().enumerate().map(
                                            |(i, it)| {
                                                let lat = it.latitude;
                                                let lng = it.longitude;
                                                let name = it.name.clone();
                                                let sub = if it.sub.is_empty() {
                                                    format!("{lat:.5}, {lng:.5}")
                                                } else {
                                                    it.sub.clone()
                                                };
                                                div()
                                                    .id(("sr-item", i))
                                                    .px(px(10.))
                                                    .py(px(7.))
                                                    .rounded(px(8.))
                                                    .flex()
                                                    .flex_col()
                                                    .gap(px(2.))
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(th.surface_hover))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.jump_to_wgs(lat, lng, 15.0, cx);
                                                            this.search_open = false;
                                                            cx.notify();
                                                        },
                                                    ))
                                                    .child(
                                                        div()
                                                            .text_size(px(12.5))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(th.text)
                                                            .child(name),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(10.5))
                                                            .text_color(th.muted)
                                                            .child(sub),
                                                    )
                                            },
                                        )),
                                )
                            }),
                    )
                    // 3c. Telemetry & Action Card
                    .child(
                        div()
                            .p(px(12.))
                            .rounded(px(14.))
                            .bg(th.card_bg)
                            .border_1()
                            .border_color(th.divider)
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            // Status row
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(7.))
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .size(px(8.))
                                                    .rounded(px(999.))
                                                    .bg(status_dot_color),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(th.text)
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .child(status_title),
                                            ),
                                    )
                                    .when_some(method_badge, |el, badge| {
                                        el.child(
                                            div()
                                                .px(px(7.))
                                                .py(px(2.))
                                                .rounded(px(999.))
                                                .bg(th.accent_soft)
                                                .text_size(px(10.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(th.accent)
                                                .child(badge),
                                        )
                                    }),
                            )
                            // Coordinate telemetry wells + copy button
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .px(px(10.))
                                            .py(px(7.))
                                            .rounded(px(9.))
                                            .bg(th.input_bg)
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.))
                                            .child(
                                                div()
                                                    .text_size(px(9.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(th.muted)
                                                    .child("LAT"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(12.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(th.text)
                                                    .child(format!("{:.5}", self.wgs_lat)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .px(px(10.))
                                            .py(px(7.))
                                            .rounded(px(9.))
                                            .bg(th.input_bg)
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.))
                                            .child(
                                                div()
                                                    .text_size(px(9.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(th.muted)
                                                    .child("LNG"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(12.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(th.text)
                                                    .child(format!("{:.5}", self.wgs_lng)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("btn-copy-coord")
                                            .w(px(34.))
                                            .h(px(44.))
                                            .rounded(px(9.))
                                            .bg(if self.copied_until.is_some() {
                                                th.green_soft
                                            } else {
                                                th.input_bg
                                            })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(13.))
                                            .text_color(if self.copied_until.is_some() {
                                                th.green
                                            } else {
                                                th.text_secondary
                                            })
                                            .cursor_pointer()
                                            .hover(|s| s.bg(th.surface_hover).text_color(th.text))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.copy_coords(cx);
                                            }))
                                            .child(if self.copied_until.is_some() {
                                                "✓"
                                            } else {
                                                "⧉"
                                            }),
                                    ),
                            )
                            // Status message
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(th.muted)
                                    .child(status_msg),
                            )
                            // Optional error box
                            .when(!self.state.error.is_empty(), |el| {
                                el.child(
                                    div()
                                        .px(px(9.))
                                        .py(px(6.))
                                        .rounded(px(8.))
                                        .bg(th.red_soft)
                                        .text_size(px(11.))
                                        .text_color(th.red)
                                        .child(self.state.error.clone()),
                                )
                            })
                            // Primary action buttons
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .id("btn-lock")
                                            .flex_1()
                                            .h(px(36.))
                                            .rounded(px(10.))
                                            .bg(if self.state.busy {
                                                th.accent.opacity(0.55)
                                            } else {
                                                th.accent
                                            })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .gap(px(5.))
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgba(0xffffffff))
                                            .cursor_pointer()
                                            .hover(|s| s.bg(th.accent_hover))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if this.state.busy {
                                                    return;
                                                }
                                                let lbl = format!(
                                                    "{:.4}, {:.4}",
                                                    this.wgs_lat, this.wgs_lng
                                                );
                                                this.lock_location(
                                                    this.wgs_lat,
                                                    this.wgs_lng,
                                                    lbl,
                                                    cx,
                                                );
                                            }))
                                            .child("⌖")
                                            .child(loc.btn_lock()),
                                    )
                                    .child(
                                        div()
                                            .id("btn-clear")
                                            .flex_1()
                                            .h(px(36.))
                                            .rounded(px(10.))
                                            .bg(if self.state.active {
                                                th.red_soft
                                            } else {
                                                th.surface
                                            })
                                            .border_1()
                                            .border_color(if self.state.active {
                                                th.red.opacity(0.35)
                                            } else {
                                                th.divider
                                            })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .gap(px(5.))
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(if self.state.active {
                                                th.red
                                            } else {
                                                th.text_secondary
                                            })
                                            .cursor_pointer()
                                            .hover(|s| s.bg(th.surface_hover))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if !this.state.busy {
                                                    this.clear_location(cx);
                                                }
                                            }))
                                            .child("↺")
                                            .child(loc.btn_clear()),
                                    ),
                            ),
                    )
                    // 3d. Quick Cities & Favorites
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(9.))
                            // Header row: segmented tabs + "+ Save"
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .p(px(3.))
                                            .rounded(px(10.))
                                            .bg(th.input_bg)
                                            .flex()
                                            .items_center()
                                            .gap(px(2.))
                                            .child(
                                                div()
                                                    .id("tab-cities")
                                                    .flex_1()
                                                    .h(px(26.))
                                                    .rounded(px(7.5))
                                                    .bg(if self.places_tab == PlacesTab::Cities {
                                                        th.surface_hover
                                                    } else {
                                                        rgba(0x00000000).into()
                                                    })
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_size(px(11.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(
                                                        if self.places_tab == PlacesTab::Cities {
                                                            th.text
                                                        } else {
                                                            th.muted
                                                        },
                                                    )
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.places_tab = PlacesTab::Cities;
                                                        cx.notify();
                                                    }))
                                                    .child(loc.tab_cities()),
                                            )
                                            .child(
                                                div()
                                                    .id("tab-favorites")
                                                    .flex_1()
                                                    .h(px(26.))
                                                    .rounded(px(7.5))
                                                    .bg(if self.places_tab == PlacesTab::Favorites {
                                                        th.surface_hover
                                                    } else {
                                                        rgba(0x00000000).into()
                                                    })
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .gap(px(4.))
                                                    .text_size(px(11.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(
                                                        if self.places_tab == PlacesTab::Favorites {
                                                            th.text
                                                        } else {
                                                            th.muted
                                                        },
                                                    )
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.places_tab = PlacesTab::Favorites;
                                                        cx.notify();
                                                    }))
                                                    .child(loc.tab_fav())
                                                    .child(
                                                        div()
                                                            .px(px(5.))
                                                            .rounded(px(999.))
                                                            .bg(th.surface)
                                                            .text_size(px(10.))
                                                            .text_color(th.muted)
                                                            .child(self.favorites.len().to_string()),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("btn-add-fav")
                                            .h(px(32.))
                                            .px(px(10.))
                                            .rounded(px(9.))
                                            .bg(th.surface)
                                            .border_1()
                                            .border_color(th.divider)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(11.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(th.text_secondary)
                                            .cursor_pointer()
                                            .hover(|s| s.bg(th.surface_hover).text_color(th.text))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.places_tab = PlacesTab::Favorites;
                                                this.fav_form_open = true;
                                                this.fav_input.update(cx, |i, cx| {
                                                    i.clear(cx);
                                                    i.focus_handle(cx).focus(window);
                                                });
                                                cx.notify();
                                            }))
                                            .child(loc.btn_add_fav()),
                                    ),
                            )
                            // Inline Add Favorite Form
                            .when(self.fav_form_open, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .child(
                                            div()
                                                .flex_1()
                                                .h(px(30.))
                                                .px(px(8.))
                                                .rounded(px(8.))
                                                .bg(th.input_bg)
                                                .border_1()
                                                .border_color(th.accent_border)
                                                .flex()
                                                .items_center()
                                                .child(self.fav_input.clone()),
                                        )
                                        .child(
                                            div()
                                                .id("btn-fav-save")
                                                .h(px(30.))
                                                .px(px(10.))
                                                .rounded(px(8.))
                                                .bg(th.accent)
                                                .flex()
                                                .items_center()
                                                .text_size(px(11.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgba(0xffffffff))
                                                .cursor_pointer()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    let name = this.fav_input.read(cx).text();
                                                    this.submit_favorite(name, cx);
                                                }))
                                                .child(loc.btn_save()),
                                        )
                                        .child(
                                            div()
                                                .id("btn-fav-cancel")
                                                .h(px(30.))
                                                .px(px(8.))
                                                .rounded(px(8.))
                                                .bg(th.surface)
                                                .flex()
                                                .items_center()
                                                .text_size(px(11.5))
                                                .text_color(th.muted)
                                                .cursor_pointer()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.fav_form_open = false;
                                                    cx.notify();
                                                }))
                                                .child(loc.btn_cancel()),
                                        ),
                                )
                            })
                            // Quick Cities 2x3 Grid
                            .when(self.places_tab == PlacesTab::Cities, |el| {
                                let rows: Vec<_> = PRESETS.chunks(2).collect();
                                el.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(7.))
                                        .children(rows.into_iter().enumerate().map(
                                            |(r_idx, chunk)| {
                                                div()
                                                    .id(("city-row", r_idx))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(7.))
                                                    .children(chunk.iter().map(|p| {
                                                        let lat = p.lat;
                                                        let lng = p.lng;
                                                        let cname = cities_names[p.idx];
                                                        let reg = p.region;
                                                        div()
                                                            .id(("city-card", p.idx))
                                                            .flex_1()
                                                            .p(px(10.))
                                                            .rounded(px(11.))
                                                            .bg(th.card_bg)
                                                            .border_1()
                                                            .border_color(th.divider)
                                                            .flex()
                                                            .items_center()
                                                            .justify_between()
                                                            .gap(px(6.))
                                                            .cursor_pointer()
                                                            .hover(|s| {
                                                                s.bg(th.surface_hover)
                                                                    .border_color(th.accent_border)
                                                            })
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.jump_to_wgs(
                                                                        lat, lng, 13.0, cx,
                                                                    );
                                                                },
                                                            ))
                                                            .child(
                                                                div()
                                                                    .flex()
                                                                    .flex_col()
                                                                    .gap(px(2.))
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(12.5))
                                                                            .font_weight(
                                                                                FontWeight::SEMIBOLD,
                                                                            )
                                                                            .text_color(th.text)
                                                                            .child(cname),
                                                                    )
                                                                    .child(
                                                                        div()
                                                                            .text_size(px(10.))
                                                                            .text_color(th.muted)
                                                                            .child(format!(
                                                                                "{lat:.2}, {lng:.2}"
                                                                            )),
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .px(px(6.))
                                                                    .py(px(2.))
                                                                    .rounded(px(5.))
                                                                    .bg(th.input_bg)
                                                                    .text_size(px(9.5))
                                                                    .font_weight(
                                                                        FontWeight::SEMIBOLD,
                                                                    )
                                                                    .text_color(th.muted)
                                                                    .child(reg),
                                                            )
                                                    }))
                                            },
                                        )),
                                )
                            })
                            // Favorites List
                            .when(self.places_tab == PlacesTab::Favorites, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.))
                                        .when(self.favorites.is_empty(), |box_el| {
                                            box_el.child(
                                                div()
                                                    .p(px(16.))
                                                    .rounded(px(11.))
                                                    .bg(th.card_bg)
                                                    .border_1()
                                                    .border_color(th.divider)
                                                    .text_size(px(11.5))
                                                    .text_color(th.muted)
                                                    .child(loc.fav_empty()),
                                            )
                                        })
                                        .children(self.favorites.iter().enumerate().map(|(i, f)| {
                                            let lat = f.latitude;
                                            let lng = f.longitude;
                                            let name = f.name.clone();
                                            let name_tp = f.name.clone();
                                            let id_del = f.id.clone();
                                            div()
                                                .id(("fav-row", i))
                                                .px(px(10.))
                                                .py(px(8.))
                                                .rounded(px(11.))
                                                .bg(th.card_bg)
                                                .border_1()
                                                .border_color(th.divider)
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .gap(px(8.))
                                                .cursor_pointer()
                                                .hover(|s| s.bg(th.surface_hover))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.jump_to_wgs(lat, lng, 15.0, cx);
                                                }))
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .flex()
                                                        .flex_col()
                                                        .gap(px(2.))
                                                        .child(
                                                            div()
                                                                .text_size(px(12.5))
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_color(th.text)
                                                                .overflow_hidden()
                                                                .whitespace_nowrap()
                                                                .child(name),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(10.))
                                                                .text_color(th.muted)
                                                                .child(format!(
                                                                    "{lat:.5}, {lng:.5}"
                                                                )),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap(px(5.))
                                                        .child(
                                                            div()
                                                                .id(("fav-tp", i))
                                                                .h(px(24.))
                                                                .px(px(8.))
                                                                .rounded(px(7.))
                                                                .bg(th.accent)
                                                                .flex()
                                                                .items_center()
                                                                .text_size(px(11.))
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_color(rgba(0xffffffff))
                                                                .cursor_pointer()
                                                                .on_click(cx.listener(
                                                                    move |this, _, _, cx| {
                                                                        cx.stop_propagation();
                                                                        this.jump_to_wgs(
                                                                            lat, lng, 15.0, cx,
                                                                        );
                                                                        this.lock_location(
                                                                            lat,
                                                                            lng,
                                                                            name_tp.clone(),
                                                                            cx,
                                                                        );
                                                                    },
                                                                ))
                                                                .child(loc.fav_teleport()),
                                                        )
                                                        .child(
                                                            div()
                                                                .id(("fav-del", i))
                                                                .size(px(24.))
                                                                .rounded(px(7.))
                                                                .bg(th.input_bg)
                                                                .flex()
                                                                .items_center()
                                                                .justify_center()
                                                                .text_size(px(12.))
                                                                .text_color(th.muted)
                                                                .cursor_pointer()
                                                                .hover(|s| {
                                                                    s.bg(th.red_soft)
                                                                        .text_color(th.red)
                                                                })
                                                                .on_click(cx.listener(
                                                                    move |this, _, _, cx| {
                                                                        cx.stop_propagation();
                                                                        this.delete_favorite(
                                                                            id_del.clone(),
                                                                            cx,
                                                                        );
                                                                    },
                                                                ))
                                                                .child("×"),
                                                        ),
                                                )
                                        })),
                                )
                            }),
                    ),
            )
            // 4. Docked Console Drawer
            .child(
                div()
                    .border_t_1()
                    .border_color(th.divider)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("btn-log-toggle")
                            .h(px(36.))
                            .px(px(14.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .cursor_pointer()
                            .hover(|s| s.bg(th.surface))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.log_drawer_open = !this.log_drawer_open;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(th.muted)
                                    .child(">_")
                                    .child(loc.logs_title())
                                    .child(
                                        div()
                                            .px(px(6.))
                                            .rounded(px(999.))
                                            .bg(th.surface)
                                            .text_size(px(10.))
                                            .text_color(th.muted)
                                            .child(self.state.logs.len().to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(th.muted)
                                    .child(if self.log_drawer_open { "⌄" } else { "⌃" }),
                            ),
                    )
                    .when(self.log_drawer_open, |el| {
                        let log_text: SharedString = if self.state.logs.is_empty() {
                            loc.logs_empty().into()
                        } else {
                            self.state
                                .logs
                                .iter()
                                .rev()
                                .take(12)
                                .cloned()
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join("\n")
                                .into()
                        };
                        el.child(
                            div()
                                .px(px(12.))
                                .pb(px(12.))
                                .child(
                                    div()
                                        .max_h(px(120.))
                                        .overflow_hidden()
                                        .p(px(8.))
                                        .rounded(px(9.))
                                        .bg(th.log_bg)
                                        .border_1()
                                        .border_color(th.divider)
                                        .text_size(px(10.5))
                                        .text_color(th.text_secondary)
                                        .child(log_text),
                                ),
                        )
                    }),
            )
    }

    fn render_collapsed_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let th = self.theme();
        div()
            .id("btn-expand")
            .absolute()
            .top(px(14.))
            .left(px(96.))
            .h(px(32.))
            .px(px(12.))
            .rounded(px(11.))
            .bg(th.pill_bg)
            .border_1()
            .border_color(th.panel_border)
            .shadow(self.pill_shadow())
            .flex()
            .items_center()
            .gap(px(7.))
            .cursor_pointer()
            .hover(|s| s.border_color(th.accent_border))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.panel_collapsed = false;
                cx.notify();
            }))
            .child(div().text_size(px(13.)).text_color(th.text).child("◧"))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(th.text)
                    .child(self.locale.expand_label()),
            )
            .when(self.state.active, |el| {
                el.child(div().size(px(7.)).rounded(px(999.)).bg(th.green))
            })
    }

    fn render_layer_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let th = self.theme();
        let loc = self.locale;
        div()
            .id("layer-bar")
            .absolute()
            .top(px(14.))
            .right(px(14.))
            .p(px(3.5))
            .rounded(px(12.))
            .bg(th.pill_bg)
            .border_1()
            .border_color(th.panel_border)
            .shadow(self.pill_shadow())
            .flex()
            .items_center()
            .gap(px(2.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .children(MapLayer::ALL.iter().enumerate().map(|(i, &layer)| {
                let active = layer == self.layer;
                div()
                    .id(("layer-btn", i))
                    .h(px(27.))
                    .px(px(10.))
                    .rounded(px(8.5))
                    .bg(if active {
                        th.accent
                    } else {
                        rgba(0x00000000).into()
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(if active {
                        rgba(0xffffffff).into()
                    } else {
                        th.text_secondary
                    })
                    .cursor_pointer()
                    .when(!active, |s| s.hover(|h| h.bg(th.surface_hover).text_color(th.text)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.layer = layer;
                        this.zoom = this.zoom.min(layer.max_zoom() as f64);
                        cx.notify();
                    }))
                    .child(layer.label(loc))
            }))
    }

    fn render_map_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let th = self.theme();
        div()
            .id("map-controls")
            .absolute()
            .bottom(px(36.))
            .right(px(14.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            // Recenter
            .child(
                div()
                    .id("btn-recenter")
                    .size(px(36.))
                    .rounded(px(12.))
                    .bg(th.pill_bg)
                    .border_1()
                    .border_color(th.panel_border)
                    .shadow(self.pill_shadow())
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(15.))
                    .text_color(if self.state.active { th.accent } else { th.text })
                    .cursor_pointer()
                    .hover(|s| s.bg(th.surface_hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.state.active {
                            this.jump_to_wgs(
                                this.state.latitude,
                                this.state.longitude,
                                this.zoom.max(15.0),
                                cx,
                            );
                            this.show_toast(this.locale.recenter_ok());
                        } else {
                            this.show_toast(this.locale.recenter_none());
                        }
                        cx.notify();
                    }))
                    .child("➤"),
            )
            // Zoom +/- group
            .child(
                div()
                    .rounded(px(12.))
                    .bg(th.pill_bg)
                    .border_1()
                    .border_color(th.panel_border)
                    .shadow(self.pill_shadow())
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("btn-zoom-in")
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(th.text)
                            .cursor_pointer()
                            .hover(|s| s.bg(th.surface_hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.zoom = (this.zoom + 1.0).min(this.layer.max_zoom() as f64);
                                cx.notify();
                            }))
                            .child("+"),
                    )
                    .child(div().h(px(1.)).bg(th.divider))
                    .child(
                        div()
                            .id("btn-zoom-out")
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(th.text)
                            .cursor_pointer()
                            .hover(|s| s.bg(th.surface_hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.zoom = (this.zoom - 1.0).max(2.5);
                                cx.notify();
                            }))
                            .child("−"),
                    ),
            )
    }

    fn render_bottom_hud(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let th = self.theme();
        div()
            .absolute()
            .bottom(px(16.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("map-hud-pill")
                    .h(px(32.))
                    .px(px(13.))
                    .rounded(px(999.))
                    .bg(th.pill_bg)
                    .border_1()
                    .border_color(th.panel_border)
                    .shadow(self.pill_shadow())
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.border_color(th.accent_border))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| this.copy_coords(cx)))
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(999.))
                            .bg(th.accent_soft)
                            .text_size(px(9.5))
                            .font_weight(FontWeight::BOLD)
                            .text_color(th.accent)
                            .child("WGS-84"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(th.text)
                            .child(format!("{:.5}, {:.5}", self.wgs_lat, self.wgs_lng)),
                    )
                    .child(div().w(px(1.)).h(px(13.)).bg(th.divider))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(th.muted)
                            .child(self.layer.label(self.locale)),
                    ),
            )
    }

    fn render_attribution(&self) -> impl IntoElement {
        let th = self.theme();
        div()
            .absolute()
            .bottom(px(6.))
            .right(px(12.))
            .px(px(6.))
            .py(px(2.))
            .rounded(px(5.))
            .bg(th.pill_bg.opacity(0.78))
            .text_size(px(10.))
            .text_color(th.muted)
            .child(self.layer.attribution())
    }

    fn render_pairing_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let th = self.theme();
        let loc = self.locale;
        let host = if self.pairing.host_name.is_empty() {
            "MacBook-Location"
        } else {
            &self.pairing.host_name
        };
        let pin_formatted = if self.pairing.pin.len() == 6 {
            format!("{} {}", &self.pairing.pin[0..3], &self.pairing.pin[3..6])
        } else {
            self.pairing.pin.clone()
        };
        let status_text: SharedString = if !self.pairing.error.is_empty() {
            self.pairing.error.clone().into()
        } else if !self.pairing.message.is_empty() {
            self.pairing.message.clone().into()
        } else {
            loc.pair_waiting(host).into()
        };
        let status_color = if !self.pairing.error.is_empty() {
            th.red
        } else if self.pairing.status == "paired" {
            th.green
        } else {
            th.muted
        };

        div()
            .id("pair-modal-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x00000066))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .w(px(384.))
                    .p(px(20.))
                    .rounded(px(20.))
                    .bg(th.panel_solid)
                    .border_1()
                    .border_color(th.panel_border)
                    .shadow(self.panel_shadow())
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .when_some(self.app_icon.clone(), |el, icon| {
                                        el.child(img(icon).size(px(30.)).rounded(px(8.)))
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.))
                                            .child(
                                                div()
                                                    .text_size(px(15.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(th.text)
                                                    .child(loc.pair_title()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(th.muted)
                                                    .child(loc.pair_subtitle()),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .id("btn-pair-close")
                                    .size(px(26.))
                                    .rounded(px(7.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(14.))
                                    .text_color(th.muted)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(th.surface_hover).text_color(th.text))
                                    .on_click(cx.listener(|this, _, _, cx| this.close_pairing(cx)))
                                    .child("×"),
                            ),
                    )
                    .child(
                        div()
                            .p(px(12.))
                            .rounded(px(12.))
                            .bg(th.card_bg)
                            .border_1()
                            .border_color(th.divider)
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(th.text_secondary)
                            .child(loc.pair_step1())
                            .child(loc.pair_step2())
                            .child(loc.pair_step3(host)),
                    )
                    .when(!self.pairing.pin.is_empty(), |el| {
                        el.child(
                            div()
                                .p(px(16.))
                                .rounded(px(14.))
                                .bg(th.accent_soft)
                                .border_1()
                                .border_color(th.accent_border)
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(th.accent)
                                        .child(loc.pair_pin_label()),
                                )
                                .child(
                                    div()
                                        .text_size(px(30.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(th.text)
                                        .child(pin_formatted),
                                ),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(status_color)
                            .flex()
                            .justify_center()
                            .child(status_text),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id("btn-pair-restart")
                                    .flex_1()
                                    .h(px(36.))
                                    .rounded(px(10.))
                                    .bg(th.surface)
                                    .border_1()
                                    .border_color(th.divider)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(th.text)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(th.surface_hover))
                                    .on_click(cx.listener(|this, _, _, cx| this.open_pairing(cx)))
                                    .child(loc.pair_restart()),
                            )
                            .child(
                                div()
                                    .id("btn-pair-cancel")
                                    .flex_1()
                                    .h(px(36.))
                                    .rounded(px(10.))
                                    .bg(th.surface)
                                    .border_1()
                                    .border_color(th.divider)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(th.text_secondary)
                                    .cursor_pointer()
                                    .hover(|s| s.bg(th.surface_hover))
                                    .on_click(cx.listener(|this, _, _, cx| this.close_pairing(cx)))
                                    .child(loc.btn_cancel()),
                            ),
                    ),
            )
    }

    fn render_toast(&self, msg: String) -> impl IntoElement {
        let th = self.theme();
        div()
            .absolute()
            .top(px(58.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .px(px(16.))
                    .py(px(8.))
                    .rounded(px(999.))
                    .bg(th.toast_bg)
                    .border_1()
                    .border_color(th.panel_border)
                    .shadow(self.pill_shadow())
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(th.toast_text)
                    .child(msg),
            )
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toast_msg = self.toast.as_ref().map(|(m, _)| m.clone());

        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .font_family(".AppleSystemUIFont")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &TogglePanel, _, cx| {
                this.panel_collapsed = !this.panel_collapsed;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.panel_collapsed = false;
                this.search_input.update(cx, |inp, cx| {
                    inp.focus_handle(cx).focus(window);
                    inp.select_all_text(cx);
                });
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SwitchLanguage, _, cx| {
                this.toggle_locale(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| {
                this.toggle_theme(cx);
            }))
            // 1. Full-bleed native Metal map canvas
            .child(self.render_map_canvas(cx))
            // 2. Floating Inspector or Collapsed Pill
            .when(!self.panel_collapsed, |el| {
                el.child(self.render_inspector(cx))
            })
            .when(self.panel_collapsed, |el| {
                el.child(self.render_collapsed_bar(cx))
            })
            // 3. Top-right Layer Bar
            .child(self.render_layer_bar(cx))
            // 4. Bottom-right Map Controls
            .child(self.render_map_controls(cx))
            // 5. Bottom-center WGS-84 HUD Pill
            .child(self.render_bottom_hud(cx))
            // 6. Attribution badge
            .child(self.render_attribution())
            // 7. Wi-Fi Pairing Modal
            .when(self.pair_modal_open, |el| {
                el.child(self.render_pairing_modal(cx))
            })
            // 8. Floating Toast
            .when_some(toast_msg, |el, msg| el.child(self.render_toast(msg)))
    }
}
