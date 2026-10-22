use gpui_kit::*;

mod app;
mod i18n;
mod opener;
mod state;
mod theme;
mod views;

use app::RootView;
use gpui_kit::component::TitleBar;

// i18n: the generated lookup machinery (the `_rust_i18n_t!` macro, backend, and
// `_rust_i18n_*` helpers) uses `crate::`-relative paths, so it must be expanded
// here at the crate root for `t!` in the view modules to resolve.
rust_i18n::i18n!("locales", fallback = "en");

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);

        let settings_path = mise_core::default_settings_path();
        let settings = if settings_path.exists() {
            mise_core::AppSettings::load(&settings_path).unwrap_or_default()
        } else {
            // First launch: follow the system language, then persist the
            // resolved locale so subsequent launches are stable.
            let settings = mise_core::AppSettings {
                locale: i18n::locale_from_system(sys_locale::get_locale().as_deref()).into(),
                ..Default::default()
            };
            let _ = settings.save(&settings_path);
            settings
        };
        i18n::init(&settings.locale);

        // The window draws its own title bar (`views::titlebar`), so the
        // system one must get out of the way: transparent title bar, the
        // native macOS traffic lights repositioned to sit in that bar, and
        // the app owning title bar dragging. The title is empty because the
        // bar no longer shows any app name — the pickers are the only
        // content next to the traffic lights.
        let window_options = WindowOptions {
            window_min_size: Some(size(px(720.), px(480.))),
            ..TitleBar::window_options()
        };

        open_window(window_options, cx, move |window, cx| {
            theme::apply_theme(&settings.theme, window, cx);
            cx.new(|cx| RootView::new(window, cx, settings, settings_path))
        })
        .expect("failed to open window");
    });
}
