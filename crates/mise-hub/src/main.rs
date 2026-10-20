use gpui_kit::*;

mod app;
mod i18n;
mod state;
mod theme;
mod views;

use app::RootView;

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
            let mut settings = mise_core::AppSettings::default();
            settings.locale = i18n::locale_from_system(sys_locale::get_locale().as_deref()).into();
            let _ = settings.save(&settings_path);
            settings
        };
        i18n::init(&settings.locale);

        open_window(WindowOptions::default(), cx, move |window, cx| {
            theme::apply_theme(&settings.theme, window, cx);
            cx.new(|cx| RootView::new(window, cx, settings, settings_path))
        })
        .expect("failed to open window");
    });
}
