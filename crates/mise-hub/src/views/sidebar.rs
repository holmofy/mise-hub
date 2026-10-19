use crate::state::{AppState, Selection};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

pub fn render(state: &AppState, cx: &mut Context<crate::app::RootView>) -> impl IntoElement {
    let theme = cx.theme();
    let mut col = div().v_flex().gap_1().p_2().w_56().h_full();

    col = col.child(section_label(t!("nav.workbench").to_string()));
    col = col.child(nav_item(t!("nav.discover").to_string(), Selection::Discover, state, theme));
    col = col.child(nav_item(t!("nav.tools").to_string(), Selection::GlobalTools, state, theme));
    col = col.child(nav_item(t!("nav.tasks").to_string(), Selection::GlobalTasks, state, theme));
    col = col.child(nav_item(t!("nav.env").to_string(), Selection::GlobalEnv, state, theme));

    col = col.child(section_label(t!("nav.projects").to_string()));
    for project in &state.projects.projects {
        col = col.child(nav_item(
            project.name.clone(),
            Selection::ProjectTools(project.id.clone()),
            state,
            theme,
        ));
    }

    col
}

fn section_label(text: String) -> impl IntoElement {
    div().px_2().pt_3().pb_1().text_xs().opacity(0.6).child(text)
}

fn nav_item(
    label: String,
    selection: Selection,
    state: &AppState,
    theme: &gpui_kit::component::theme::Theme,
) -> impl IntoElement {
    let selected = state.selection == selection;
    div()
        .px_2()
        .py_1()
        .rounded_md()
        .when(selected, |d| d.bg(theme.muted).text_color(theme.foreground))
        .child(label)
}
