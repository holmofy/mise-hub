use crate::app::RootView;
use crate::state::{RunStatus, TerminalState};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

/// Height of the expanded panel. Collapsed shows only its header bar.
const PANEL_HEIGHT: f32 = 220.0;
const HEADER_HEIGHT: f32 = 30.0;

/// Render the bottom terminal panel: one tab per run, the active tab
/// highlighted, a spinner + running marker on live sessions, and 保存 /
/// 复制 / 清除 actions. `TerminalState.visible` toggles between the full
/// ~220px panel and the collapsed header bar; cleared sessions keep their
/// tab. Lines render in a monospace font; stdout in the normal foreground
/// color, stderr in the error color.
pub fn render(state: &TerminalState, cx: &mut Context<RootView>) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();
    let active = state.active;
    let collapsed = !state.visible;

    // Tab bar: one entry per session (bounded by real runs, so a plain loop
    // is fine here — tabs are not a virtual-list concern).
    let mut tabs = div().h_flex().flex_1().min_w_0().gap_1().overflow_hidden();
    for (ix, session) in state.sessions.iter().enumerate() {
        let running = session.status == RunStatus::Running;
        let label = if running {
            format!("⏳ {}", session.title)
        } else {
            session.title.clone()
        };
        let status: Option<String> = match &session.status {
            RunStatus::Running => Some(t!("terminal.running").to_string()),
            RunStatus::Succeeded => Some(t!("terminal.succeeded").to_string()),
            RunStatus::Failed(code) => Some(format!("{} ({code})", t!("terminal.failed"))),
        };
        tabs = tabs.child(
            div()
                .id(("terminal-tab", ix))
                .h_flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .text_xs()
                .min_w_0()
                .max_w_48()
                .cursor_pointer()
                .border_b_1()
                .border_color(if ix == active {
                    theme.primary
                } else {
                    theme.border
                })
                .when(ix != active, |d| {
                    d.text_color(theme.muted_foreground)
                        .hover(|d| d.bg(theme.muted.opacity(0.6)))
                })
                .when(ix == active, |d| d.bg(theme.muted.opacity(0.4)))
                .child(div().min_w_0().text_ellipsis().child(label))
                .when(ix == state.active && running, |d| {
                    d.child(Spinner::new().xsmall())
                })
                .when_some(status, |d, status| {
                    d.child(
                        div()
                            .text_color(if running {
                                theme.muted_foreground
                            } else {
                                match ix == active {
                                    true => theme.foreground,
                                    false => theme.muted_foreground,
                                }
                            })
                            .when(matches!(session.status, RunStatus::Failed(_)), |d| {
                                d.text_color(theme.danger)
                            })
                            .min_w_0()
                            .child(status),
                    )
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.state.terminal.select(ix);
                    this.state.terminal.visible = true;
                    cx.notify();
                })),
        );
    }

    // Actions row.
    let actions = div()
        .h_flex()
        .items_center()
        .gap_1()
        .child(
            Button::new("terminal-save")
                .ghost()
                .xsmall()
                .label(t!("terminal.save").to_string())
                .tooltip(t!("terminal.save_note").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.save_terminal_log(cx);
                })),
        )
        .child(
            Button::new("terminal-copy")
                .ghost()
                .xsmall()
                .label(t!("terminal.copy").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.copy_terminal(cx);
                })),
        )
        .child(
            Button::new("terminal-clear")
                .ghost()
                .xsmall()
                .label(t!("terminal.clear").to_string())
                .tooltip(t!("terminal.clear_keeps_tab").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.state.terminal.clear_active();
                    cx.notify();
                })),
        );

    // Toggle collapse/expand.
    let toggle = Button::new("terminal-toggle")
        .ghost()
        .xsmall()
        .label(if collapsed {
            t!("terminal.expand").to_string()
        } else {
            t!("terminal.collapse").to_string()
        })
        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
            this.state.terminal.visible = !this.state.terminal.visible;
            cx.notify();
        }));

    let header = div()
        .h_flex()
        .w_full()
        .px_2()
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.muted.opacity(0.3))
        .child(tabs)
        .child(actions)
        .child(toggle);

    let body = div().flex_col().size_full().when(!collapsed, |d| {
        let mut content = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .px_3()
            .py_1()
            .font_family(crate::theme::monospace_family())
            .text_xs();
        if let Some(session) = state.active_session() {
            for line in &session.lines {
                content = content.child(
                    div()
                        .whitespace_nowrap()
                        .text_color(match line.stream {
                            crate::state::Stream::Stdout => theme.foreground,
                            crate::state::Stream::Stderr => theme.danger,
                        })
                        .child(line.text.clone()),
                );
            }
        }
        d.child(content)
    });

    if collapsed {
        div()
            .v_flex()
            .w_full()
            .h(px(HEADER_HEIGHT))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(header)
            .into_any_element()
    } else {
        div()
            .v_flex()
            .w_full()
            .h(px(PANEL_HEIGHT))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(header)
            .child(body)
            .into_any_element()
    }
}
