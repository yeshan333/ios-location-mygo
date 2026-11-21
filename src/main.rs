mod coords;
mod i18n;
mod spoofer;
mod storage;
mod text_input;
mod theme;
mod ui;
mod tiles;

use gpui::{
    App, Application, Bounds, Focusable, KeyBinding, TitlebarOptions, WindowAppearance,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, point, prelude::*, px, size,
};
use i18n::Locale;
use spoofer::SpooferController;
use storage::{app_data_dir, load_settings_from};
use text_input::bind_text_input_keys;
use ui::{AppView, FocusSearch, QuitApp, TogglePanel, install_app_menus};

fn main() {
    let data_dir = app_data_dir();
    let settings = load_settings_from(&data_dir);
    let initial_locale = settings.locale.unwrap_or_else(Locale::detect_system);
    let spoofer = SpooferController::new(initial_locale);
    let spoofer_quit = spoofer.clone();

    Application::new().run(move |cx: &mut App| {
        cx.activate(true);
        bind_text_input_keys(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-q", QuitApp, None),
            KeyBinding::new("cmd-k", FocusSearch, None),
            KeyBinding::new("cmd-\\", TogglePanel, None),
        ]);

        let spoofer_for_action = spoofer_quit.clone();
        cx.on_action(move |_: &QuitApp, cx: &mut App| {
            spoofer_for_action.shutdown();
            cx.quit();
        });

        let spoofer_for_close = spoofer_quit.clone();
        cx.on_window_closed(move |cx| {
            if cx.windows().is_empty() {
                spoofer_for_close.shutdown();
                cx.quit();
            }
        })
        .detach();

        install_app_menus(initial_locale, cx);

        let bounds = Bounds::centered(None, size(px(1180.), px(780.)), cx);
        let dark_override = settings.dark_mode;
        let spoofer_for_win = spoofer.clone();

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(820.), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(initial_locale.app_title().into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(26.), px(24.))),
                }),
                window_background: WindowBackgroundAppearance::Blurred,
                ..Default::default()
            },
            move |window, cx| {
                let dark = dark_override.unwrap_or_else(|| {
                    matches!(
                        window.appearance(),
                        WindowAppearance::Dark | WindowAppearance::VibrantDark
                    )
                });
                let view = cx.new(|cx| AppView::new(initial_locale, dark, spoofer_for_win.clone(), cx));
                let fh = view.read(cx).focus_handle(cx);
                window.focus(&fh);
                view
            },
        )
        .expect("failed to open GPUI window");
    });
}
