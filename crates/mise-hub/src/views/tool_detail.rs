use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Disableable;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::*;
use gpui_kit::*;
use mise_core::{Scope, ToolRow};
use rust_i18n::t;

/// Render the detail pane for a single installed tool row.
///
/// M1 is read-only: there are no apply/uninstall actions. The 「应用版本」 button
/// is rendered disabled and labeled as M2 so the future action slot is visible.
pub fn render(row: &ToolRow, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();

    div()
        .flex_1()
        .h_full()
        .v_flex()
        .p_4()
        .gap_3()
        .bg(theme.background)
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(div().text_lg().font_semibold().child(row.name.clone()))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(format!("{} {}", t!("tools.version"), row.version)),
                ),
        )
        .child(field(
            cx,
            t!("tools.scope").to_string(),
            scope_label(row.scope),
        ))
        .child(field(
            cx,
            t!("tools.source").to_string(),
            row.source_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "—".to_string()),
        ))
        .child(field(
            cx,
            t!("tools.install_path").to_string(),
            row.install_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "—".to_string()),
        ))
        .child(field(cx, t!("tools.active").to_string(), active_label(row)))
        .child(div().flex_1())
        .child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(
                    Button::new("apply")
                        .primary()
                        .disabled(true)
                        .label(t!("tools.apply").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!("tools.apply_m2_note").to_string()),
                ),
        )
}

fn field(cx: &App, label: String, value: String) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .v_flex()
        .gap_0p5()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(div().text_sm().child(value))
}

fn scope_label(scope: Scope) -> String {
    match scope {
        Scope::Project => t!("tools.project").to_string(),
        Scope::Global => t!("tools.global").to_string(),
    }
}

fn active_label(row: &ToolRow) -> String {
    if row.active {
        t!("tools.active").to_string()
    } else {
        t!("tools.inactive").to_string()
    }
}
