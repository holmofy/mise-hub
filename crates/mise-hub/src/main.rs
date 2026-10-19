use gpui_kit::*;

mod app;
mod i18n;
mod state;
mod views;

use app::RootView;

// i18n: the generated lookup machinery (the `_rust_i18n_t!` macro, backend, and
// `_rust_i18n_*` helpers) uses `crate::`-relative paths, so it must be expanded
// here at the crate root for `t!` in the view modules to resolve.
rust_i18n::i18n!("locales", fallback = "en");

fn main() {
    application()
        .with_assets(assets::Assets)
        .run(|cx| {
            init(cx);
            i18n::init("en");
            open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| RootView::new(window, cx))
            })
            .expect("failed to open window");
        });
}
