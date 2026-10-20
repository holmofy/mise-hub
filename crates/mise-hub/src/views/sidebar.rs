use crate::app::RootView;
use crate::state::Selection;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;
use mise_core::Project;
use rust_i18n::t;

pub fn render(
    cx: &mut Context<RootView>,
    projects: &[Project],
    selection: &Selection,
    collapsed: bool,
) -> impl IntoElement {
    let theme = cx.theme().clone();

    let mut col = div()
        .v_flex()
        .gap_1()
        .p_2()
        .w_56()
        .h_full()
        .border_r_1()
        .border_color(theme.border);

    col = col.child(section_label(t!("nav.workbench").to_string()));
    col = col.child(nav_item(
        cx,
        t!("nav.discover").to_string(),
        Selection::Discover,
        selection,
        &theme,
    ));
    col = col.child(nav_item(
        cx,
        t!("nav.tools").to_string(),
        Selection::GlobalTools,
        selection,
        &theme,
    ));
    col = col.child(nav_item(
        cx,
        t!("nav.tasks").to_string(),
        Selection::GlobalTasks,
        selection,
        &theme,
    ));
    col = col.child(nav_item(
        cx,
        t!("nav.env").to_string(),
        Selection::GlobalEnv,
        selection,
        &theme,
    ));

    col = col.child(project_header(cx, collapsed, &theme));

    if !collapsed {
        for project in projects {
            col = col.child(project_item(cx, project, selection, &theme));
        }
    }

    col
}

fn section_label(text: String) -> impl IntoElement {
    div()
        .px_2()
        .pt_3()
        .pb_1()
        .text_xs()
        .opacity(0.6)
        .child(text)
}

fn nav_item(
    cx: &mut Context<RootView>,
    label: String,
    selection: Selection,
    current: &Selection,
    theme: &Theme,
) -> impl IntoElement {
    let selected = current == &selection;
    // Click handling lives on `StatefulInteractiveElement`, so the div needs an
    // element id to become stateful first.
    let id = format!("nav-{}", selection_id(&selection));
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|d| d.bg(theme.muted.opacity(0.6)))
        .when(selected, |d| d.bg(theme.muted).text_color(theme.foreground))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.select(selection.clone(), cx);
        }))
        .child(label)
}

/// A stable, unique string for one `Selection`, used to build element ids.
fn selection_id(selection: &Selection) -> String {
    match selection {
        Selection::Discover => "discover".into(),
        Selection::GlobalTools => "global-tools".into(),
        Selection::GlobalTasks => "global-tasks".into(),
        Selection::GlobalEnv => "global-env".into(),
        Selection::ProjectTools(id) => format!("project-tools-{}", id),
        Selection::ProjectTasks(id) => format!("project-tasks-{}", id),
        Selection::ProjectEnv(id) => format!("project-env-{}", id),
    }
}

fn project_header(cx: &mut Context<RootView>, collapsed: bool, theme: &Theme) -> impl IntoElement {
    div()
        .h_flex()
        .items_center()
        .justify_between()
        .px_2()
        .pt_3()
        .pb_1()
        .child(
            div()
                .id("projects-toggle")
                .h_flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.projects_collapsed = !this.projects_collapsed;
                    cx.notify();
                }))
                .child(
                    Icon::new(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .size_3p5()
                    .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .opacity(0.6)
                        .child(t!("nav.projects").to_string()),
                ),
        )
        .child(
            Button::new("add-project")
                .ghost()
                .xsmall()
                .icon(IconName::Plus)
                .tooltip(t!("nav.add_project").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.prompt_add_project(cx);
                })),
        )
}

fn project_item(
    cx: &mut Context<RootView>,
    project: &Project,
    selection: &Selection,
    theme: &Theme,
) -> impl IntoElement {
    let id = project.id.clone();
    let name = project.name.clone();
    let selected = matches!(selection, Selection::ProjectTools(sel) if sel == &id);
    let click_id = id.clone();

    div()
        .h_flex()
        .items_center()
        .rounded_md()
        .hover(|d| d.bg(theme.muted.opacity(0.6)))
        .when(selected, |d| d.bg(theme.muted))
        .child(
            div()
                .id(format!("project-row-{}", id))
                .h_flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .flex_1()
                .px_2()
                .py_1()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.select(Selection::ProjectTools(click_id.clone()), cx);
                }))
                .child(
                    div()
                        .text_sm()
                        .min_w_0()
                        .text_ellipsis()
                        .child(name.clone()),
                ),
        )
        .child(
            Button::new(format!("remove-project-{}", id))
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .tooltip(t!("nav.remove_project").to_string())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.prompt_remove_project(id.clone(), name.clone(), window, cx);
                })),
        )
}
