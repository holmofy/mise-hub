use crate::app::RootView;
use crate::state::Loadable;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::v_virtual_list;
use gpui_kit::component::{Disableable, Sizable, VirtualListScrollHandle};
use gpui_kit::prelude::*;
use gpui_kit::*;
use mise_core::SearchHit;
use mise_core::ToolEntry;
use rust_i18n::t;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

/// Fixed row height. The virtual list lays each row out with the height
/// given in `item_sizes`, so rows must not grow beyond this; single-line
/// text with `text_ellipsis` gives the exact accounting:
/// py_2(16) + text_sm line(20) + mt_1(4) + text_sm line(20)
/// + mt_1(4) + text_xs line(16) = 80.
const ROW_HEIGHT: f32 = 80.0;

/// Render the discover page: a search box over the local registry index,
/// and the results list. `hits` is already searched/browsed by the caller
/// (empty query = all entries alphabetically, see `AppState::search`), and
/// stays alive on `RootView::discover_hits` so the virtual list closure can
/// slice it per frame without re-running the search. `installed_names` are
/// the tool names from `mise ls` used to mark already-installed rows.
/// `registry` carries the load state: `Loading` / `Idle` show progress and
/// `Failed` is shown raw (AGENTS.md rule 9).
pub fn render(
    cx: &mut Context<RootView>,
    search_input: &Entity<InputState>,
    hits: &[SearchHit],
    installed_names: &HashSet<String>,
    scrolling: &VirtualListScrollHandle,
    registry: &Loadable<Vec<ToolEntry>>,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();

    let mut col = div()
        .v_flex()
        .flex_1()
        .min_h_0()
        .h_full()
        .bg(theme.background);

    col = col.child(search_bar(cx, search_input));

    match registry {
        Loadable::Failed(err) => col = col.child(error_panel(cx, err.as_str())),
        // `Idle` loads too: mise was only just detected (setup page path).
        Loadable::Idle | Loadable::Loading => {
            col = col.child(centered(&theme, t!("discover.loading").to_string()).into_any_element())
        }
        // Only reachable when a non-empty query matched nothing: browse mode
        // (empty query) always yields the full registry.
        Loadable::Ready(_) if hits.is_empty() => {
            col = col
                .child(centered(&theme, t!("discover.no_results").to_string()).into_any_element())
        }
        Loadable::Ready(_) => {
            let item_sizes = Rc::new(vec![Size::new(px(0.), px(ROW_HEIGHT)); hits.len()]);
            let installed = installed_names.clone();
            col = col.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .overflow_hidden()
                    .child(
                        v_virtual_list(
                            cx.entity(),
                            "discover-list",
                            item_sizes,
                            move |this, range: Range<usize>, _window, cx| {
                                range
                                    .map(|ix| {
                                        // Same vec as `hits` above, kept on the
                                        // view; `get` guards against any size drift.
                                        this.discover_hits.get(ix).map_or_else(
                                            || div().h(px(ROW_HEIGHT)).into_any_element(),
                                            |hit| {
                                                let selected = this
                                                    .state
                                                    .selected_tool
                                                    .as_deref()
                                                    .is_some_and(|s| s == hit.tool.short);
                                                row(
                                                    cx,
                                                    hit,
                                                    installed.contains(&hit.tool.short),
                                                    selected,
                                                )
                                                .into_any_element()
                                            },
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            },
                        )
                        .track_scroll(scrolling)
                        .into_any_element(),
                    )
                    .child(Scrollbar::vertical(scrolling)),
            );
        }
    }

    col
}

fn search_bar(cx: &mut Context<RootView>, search_input: &Entity<InputState>) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .v_flex()
        .gap_1()
        .p_2()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .h_flex()
                .gap_2()
                // The input renders on its own row and the deep-search entry
                // wraps with the note; both must always be visible.
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(search_input).cleanable(true).small()),
                )
                .child(
                    // Disabled on purpose: deep search is `mise search --all`
                    // (online) and lands in M2. Not rendered as a dead button —
                    // the note below explains it (AGENTS.md red line).
                    Button::new("deep-search")
                        .outline()
                        .small()
                        .disabled(true)
                        .label(t!("discover.deep_search").to_string())
                        .tooltip(t!("discover.deep_search_note").to_string()),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("discover.search_note").to_string()),
        )
}

fn row(
    cx: &mut Context<RootView>,
    hit: &SearchHit,
    installed: bool,
    selected: bool,
) -> impl IntoElement {
    let theme = cx.theme();
    let name = hit.tool.short.clone();
    let click_name = name.clone();
    let backends = hit.tool.backends.join(", ");
    let bins = hit.tool.bins.join(" · ");
    // Description may be missing; keep the row layout stable regardless.
    let description = hit.tool.description.clone().unwrap_or_default();

    div()
        .id(name.clone())
        .h(px(ROW_HEIGHT))
        .w_full()
        .px_3()
        .py_2()
        .overflow_hidden()
        .cursor_pointer()
        .hover(|d| d.bg(theme.muted.opacity(0.6)))
        .when(selected, |d| d.bg(theme.muted))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.state.selected_tool = Some(click_name.clone());
            cx.notify();
        }))
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(div().text_sm().font_medium().child(name.clone()))
                        .when(!backends.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .min_w_0()
                                    .text_ellipsis()
                                    .child(backends),
                            )
                        }),
                )
                .when(installed, |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme.success)
                            .child(t!("discover.installed").to_string()),
                    )
                }),
        )
        .when(!description.is_empty(), |d| {
            d.child(
                div()
                    .mt_1()
                    .text_sm()
                    .text_color(theme.foreground.opacity(0.8))
                    .min_w_0()
                    .text_ellipsis()
                    .child(description),
            )
        })
        .when(!bins.is_empty(), |d| {
            d.child(
                div()
                    .mt_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .min_w_0()
                    .text_ellipsis()
                    .child(bins),
            )
        })
}

fn centered(theme: &Theme, text: String) -> impl IntoElement {
    div()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .flex()
        .text_color(theme.muted_foreground)
        .text_sm()
        .child(text)
}

fn error_panel(cx: &Context<RootView>, err: &str) -> impl IntoElement {
    let theme = cx.theme();
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
                .child(t!("discover.failed").to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(err.to_string()),
        )
}
