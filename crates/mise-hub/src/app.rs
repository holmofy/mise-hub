use crate::state::AppState;
use crate::views::sidebar;
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

pub struct RootView {
    pub state: AppState,
}

impl RootView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let path = mise_core::default_store_path();
        let projects = mise_core::ProjectStore::load(&path).unwrap_or_default();
        Self { state: AppState::new(projects) }
    }
}

impl Render for RootView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(sidebar::render(&self.state, cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .p_4()
                    .child(format!("{:?}", self.state.selection)),
            )
    }
}
