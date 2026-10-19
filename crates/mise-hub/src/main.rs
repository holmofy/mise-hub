use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

struct RootView;

impl Render for RootView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child("mise-hub")
            .child(
                Button::new("ping")
                    .primary()
                    .label("Ping")
                    .on_click(|_, _, _| println!("pong")),
            )
    }
}

fn main() {
    application()
        .with_assets(assets::Assets)
        .run(|cx| {
            init(cx);
            open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| RootView))
                .expect("failed to open window");
        });
}
