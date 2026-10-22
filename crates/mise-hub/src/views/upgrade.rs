use crate::app::RootView;
use crate::state::Loadable;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{Icon, IconName, Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;
use mise_core::OutdatedTool;
use rust_i18n::t;

/// Pure formatting predicate for one row's version transition: prefer the
/// `requested` range (what the config asks for) and fall back to `current`
/// (what is installed) when mise reports a range-less entry. Unknown halves
/// render as "?" so a never-installed tool still shows an upgradeable row.
/// Returns `(from, to)` so the render path stays free of logic and the tests
/// need no GUI or locale context.
pub(crate) fn version_range(
    requested: Option<&str>,
    current: Option<&str>,
    latest: Option<&str>,
) -> (String, String) {
    let from = requested
        .or(current)
        .filter(|s| !s.is_empty())
        .unwrap_or("?")
        .to_string();
    let to = latest.filter(|s| !s.is_empty()).unwrap_or("?").to_string();
    (from, to)
}

/// Render the upgrade page: header (count + 「全部升级」 + bump toggle),
/// then one row per outdated tool, or the loading / empty / failure state
/// (raw error retained, AGENTS.md rule 9).
pub fn render(
    cx: &mut Context<RootView>,
    outdated: &Loadable<Vec<OutdatedTool>>,
    bump: bool,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();

    let mut col = div()
        .v_flex()
        .flex_1()
        .h_full()
        .min_w_0()
        .p_3()
        .gap_2()
        .bg(theme.background);

    let count = outdated.ready().map(|v| v.len()).unwrap_or(0);
    let all_button = Button::new("upgrade-all")
        .primary()
        .small()
        .label(t!("upgrade.all").to_string())
        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
            this.run_upgrade(None, this.upgrade_bump, cx);
        }));
    let bump_toggle = Button::new("upgrade-bump")
        .outline()
        .small()
        .selected(bump)
        .label(t!("upgrade.bump").to_string())
        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
            this.upgrade_bump = !this.upgrade_bump;
            cx.notify();
        }));

    col = col.child(
        div()
            .h_flex()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(t!("upgrade.count", n = count).to_string()),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(bump_toggle)
                    // Refreshing an empty list into a run is harmless but
                    // confusing; only offer the bulk action when it has work.
                    .when(count > 0, |d| d.child(all_button)),
            ),
    );

    match outdated {
        Loadable::Failed(err) => col = col.child(failed_body(cx, err)),
        Loadable::Loading => {
            col = col.child(status_body(&theme, t!("upgrade.loading").to_string()))
        }
        Loadable::Ready(tools) if tools.is_empty() => {
            col = col.child(status_body(&theme, t!("upgrade.empty").to_string()))
        }
        Loadable::Ready(tools) => {
            let mut list = div().flex_1().min_h_0().overflow_y_scrollbar().flex_col();
            for tool in tools {
                list = list.child(row_element(cx, tool));
            }
            col = col.child(list);
        }
        Loadable::Idle => col = col.child(status_body(&theme, t!("upgrade.loading").to_string())),
    }

    col
}

fn status_body(theme: &Theme, message: String) -> gpui::AnyElement {
    div()
        .flex_1()
        .items_center()
        .justify_center()
        .p_4()
        .text_color(theme.muted_foreground)
        .text_sm()
        .child(message)
        .into_any_element()
}

fn failed_body(cx: &mut Context<RootView>, err: &str) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    div()
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .p_3()
        .v_flex()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(theme.danger)
                .child(t!("upgrade.failed").to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(err.to_string()),
        )
        .child(retry_button(cx))
        .into_any_element()
}

fn retry_button(cx: &mut Context<RootView>) -> impl IntoElement {
    Button::new("upgrade-retry")
        .outline()
        .small()
        .label(t!("common.retry").to_string())
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.state.outdated = Loadable::Loading;
            this.refresh_outdated();
            cx.notify();
        }))
}

fn row_element(cx: &mut Context<RootView>, tool: &OutdatedTool) -> impl IntoElement {
    let theme = cx.theme();
    let name = tool.name.clone();
    let (from, to) = version_range(
        tool.requested.as_deref(),
        tool.current.as_deref(),
        tool.latest.as_deref(),
    );
    let source = tool
        .source_path
        .as_deref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".to_string());

    div()
        .h_flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_3()
        .py_2()
        // Hairline separator between rows so the outdated list reads as
        // distinct entries, not one continuous block.
        .border_b_1()
        .border_color(theme.border.opacity(0.6))
        .rounded_md()
        .hover(|d| d.bg(theme.muted.opacity(0.4)))
        .child(
            div()
                .v_flex()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .h_flex()
                        .items_baseline()
                        .gap_2()
                        .child(div().text_sm().font_medium().child(tool.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!("{from} → {to}")),
                        ),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(
                            Icon::new(IconName::File)
                                .size_3()
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .text_ellipsis()
                                .overflow_hidden()
                                .child(t!("upgrade.source").to_string() + ": " + &source),
                        ),
                ),
        )
        .child(
            Button::new(format!("upgrade-row-{name}"))
                .outline()
                .small()
                .label(t!("upgrade.row").to_string())
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.run_upgrade(Some(name.clone()), this.upgrade_bump, cx);
                })),
        )
}

#[cfg(test)]
mod tests {
    use super::version_range;

    #[test]
    fn prefers_requested_over_current() {
        assert_eq!(
            version_range(Some("20"), Some("20.10.0"), Some("20.20.2")),
            ("20".into(), "20.20.2".into())
        );
    }

    #[test]
    fn falls_back_to_current_when_requested_absent() {
        assert_eq!(
            version_range(None, Some("3.12"), Some("3.13")),
            ("3.12".into(), "3.13".into())
        );
    }

    // mise can emit nulls for every field; the row must survive rendering.
    #[test]
    fn unknown_halves_render_as_placeholder() {
        assert_eq!(version_range(None, None, None), ("?".into(), "?".into()));
        assert_eq!(
            version_range(None, None, Some("9")),
            ("?".into(), "9".into())
        );
    }
}
