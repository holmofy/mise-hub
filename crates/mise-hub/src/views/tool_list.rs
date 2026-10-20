use crate::app::RootView;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;
use mise_core::{Scope, ScopeFilter, ToolRow};
use rust_i18n::t;

/// A unique key identifying a row within the current list: name + version + scope.
///
/// The same tool can appear twice (once per scope) when a project tool
/// overrides a global one, so the name alone is not enough to select a row.
pub fn row_key(row: &ToolRow) -> String {
    format!("{}|{}|{:?}", row.name, row.version, row.scope)
}

/// Render the installed-tools list: search box, optional scope filter, and the
/// clickable rows. `rows` is already filtered (scope + search query) by the
/// caller; `show_filter` decides whether the scope segmented control renders
/// (project tools page only). `error` replaces the rows with the raw failure
/// output when loading `mise ls` went wrong (AGENTS.md rule 9: show the real
/// error, never a bare "failed").
pub fn render(
    cx: &mut Context<RootView>,
    search_input: &Entity<InputState>,
    rows: Vec<ToolRow>,
    selected_key: Option<&str>,
    filter: ScopeFilter,
    show_filter: bool,
    error: Option<&str>,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();

    let mut col = div()
        .v_flex()
        .w_72()
        .h_full()
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.background);

    col = col.child(
        div()
            .p_2()
            .border_b_1()
            .border_color(theme.border)
            .child(Input::new(search_input).cleanable(true).small()),
    );

    if show_filter {
        col = col.child(scope_filter(cx, filter));
    }

    if let Some(err) = error {
        col = col.child(
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
                        .child(t!("tools.failed").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(err.to_string()),
                ),
        );
    } else if rows.is_empty() {
        col = col.child(
            div()
                .flex_1()
                .items_center()
                .justify_center()
                .p_4()
                .text_color(theme.muted_foreground)
                .text_sm()
                .child(t!("tools.empty").to_string()),
        );
    } else {
        // A tool present in both scopes shows as two rows; the non-active one
        // is genuinely overridden by its counterpart. A tool that is merely
        // installed without being active anywhere is "inactive", not overridden.
        let overridden_names: std::collections::HashSet<&str> = rows
            .iter()
            .filter(|r| !r.active)
            .filter(|r| {
                rows.iter()
                    .any(|o| o.name == r.name && o.scope != r.scope && o.active)
            })
            .map(|r| r.name.as_str())
            .collect();

        let mut list = div().flex_1().min_h_0().flex_col();
        for row in &rows {
            list = list.child(row_element(
                cx,
                row,
                selected_key,
                overridden_names.contains(row.name.as_str()),
            ));
        }
        col = col.child(list.overflow_y_scrollbar());
    }

    col
}

fn scope_filter(cx: &mut Context<RootView>, filter: ScopeFilter) -> impl IntoElement {
    div()
        .h_flex()
        .gap_1()
        .p_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(filter_button(
            cx,
            ScopeFilter::All,
            t!("tools.all").to_string(),
            filter,
        ))
        .child(filter_button(
            cx,
            ScopeFilter::Project,
            t!("tools.project").to_string(),
            filter,
        ))
        .child(filter_button(
            cx,
            ScopeFilter::Global,
            t!("tools.global").to_string(),
            filter,
        ))
}

fn filter_button(
    cx: &mut Context<RootView>,
    value: ScopeFilter,
    label: String,
    current: ScopeFilter,
) -> impl IntoElement {
    Button::new(format!("filter-{:?}", value))
        .ghost()
        .small()
        .selected(current == value)
        .label(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.tool_filter = value;
            cx.notify();
        }))
}

fn row_element(
    cx: &mut Context<RootView>,
    row: &ToolRow,
    selected_key: Option<&str>,
    overridden: bool,
) -> impl IntoElement {
    let key = row_key(row);
    let selected = selected_key == Some(key.as_str());
    let theme = cx.theme();

    div()
        .id(key.clone())
        .px_3()
        .py_2()
        .cursor_pointer()
        .hover(|d| d.bg(theme.muted.opacity(0.6)))
        .when(selected, |d| d.bg(theme.muted))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.state.selected_tool = Some(key.clone());
            cx.notify();
        }))
        .child(
            div()
                .h_flex()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .min_w_0()
                        .child(div().text_sm().font_medium().child(row.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(row.version.clone()),
                        ),
                )
                .child(scope_tag(cx, row.scope)),
        )
        .child(
            div()
                .mt_1()
                .h_flex()
                .items_center()
                .gap_1p5()
                .child(active_marker(cx, row, overridden)),
        )
}

fn scope_tag(cx: &mut Context<RootView>, scope: Scope) -> impl IntoElement {
    let theme = cx.theme();
    let (label, color) = match scope {
        Scope::Project => (t!("tools.project").to_string(), theme.info),
        Scope::Global => (t!("tools.global").to_string(), theme.muted_foreground),
    };
    div()
        .px_1p5()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .border_1()
        .border_color(color.opacity(0.4))
        .text_color(color)
        .child(label)
}

fn active_marker(cx: &Context<RootView>, row: &ToolRow, overridden: bool) -> impl IntoElement {
    let theme = cx.theme();
    if row.active {
        div()
            .h_flex()
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(theme.success)
            .child(t!("tools.active").to_string())
    } else if overridden {
        div()
            .h_flex()
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(t!("tools.overridden").to_string())
    } else {
        div()
            .h_flex()
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(t!("tools.inactive").to_string())
    }
}
