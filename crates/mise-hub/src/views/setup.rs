use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use rust_i18n::t;

pub fn render(
    on_detect: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(div().text_xl().child(t!("setup.title").to_string()))
        .child(div().max_w_96().child(t!("setup.body").to_string()))
        .child(
            Button::new("detect")
                .primary()
                .label(t!("setup.detect").to_string())
                .on_click(on_detect),
        )
}
