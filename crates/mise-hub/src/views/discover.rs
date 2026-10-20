use crate::app::RootView;
use crate::state::Loadable;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::v_virtual_list;
use gpui_kit::component::{Disableable, Sizable, VirtualListScrollHandle};
use gpui_kit::prelude::*;
use gpui_kit::*;
use mise_core::RemoteVersion;
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

fn versions_pane(
    cx: &mut Context<RootView>,
    tool: Option<&str>,
    versions: &Loadable<Vec<RemoteVersion>>,
    not_active_version: Option<String>,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();

    let mut pane = div()
        .v_flex()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .border_l_1()
        .border_color(theme.border)
        .p_3()
        .gap_2()
        .bg(theme.background);

    let title = match tool {
        Some(tool) => tool.to_string(),
        None => t!("discover.select_to_see_versions").to_string(),
    };
    pane = pane.child(div().text_sm().font_medium().child(title));

    pane = pane.child(match (tool, versions) {
        (_, Loadable::Loading) => div()
            .h_flex()
            .gap_2()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(Spinner::new().xsmall())
            .child(t!("discover.loading_versions").to_string())
            .into_any_element(),
        (Some(_tool), Loadable::Failed(err)) => div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(t!("discover.versions_failed").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(err.clone()),
            )
            .into_any_element(),
        (Some(tool), Loadable::Ready(list)) => {
            let mut col = div()
                .v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .gap_1();
            for version in list {
                let version_text = version.version.clone();
                let tool = tool.to_string();
                col = col.child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_family("Menlo")
                                .min_w_0()
                                .text_ellipsis()
                                .child(version_text.clone()),
                        )
                        .child(
                            Button::new(SharedString::from(format!("install-{version_text}")))
                                .primary()
                                .xsmall()
                                .label(t!("discover.install").to_string())
                                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                    this.run_install(&tool, &version_text, cx);
                                })),
                        ),
                );
            }
            col.into_any_element()
        }
        (Some(_), Loadable::Idle) => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(t!("discover.loading_versions").to_string())
            .into_any_element(),
        // No tool picked yet.
        (None, _) => div()
            .v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(t!("discover.pick_tool").to_string())
            .into_any_element(),
    });

    // Rule 4: installing and activating are separate actions.
    // After an install the tool is on the machine but NOT active; the apply
    // affordance is honest and explicitly not implemented in M1.
    if let Some(version) = not_active_version {
        pane = pane.child(
            div()
                .v_flex()
                .gap_1()
                .mt_1()
                .border_t_1()
                .border_color(theme.border)
                .pt_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.success)
                        .child(t!("discover.installed_not_active").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} {}",
                            t!("discover.installed_at_version"),
                            version
                        )),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("apply-project")
                                .outline()
                                .small()
                                .disabled(true)
                                .label(t!("discover.apply_project").to_string()),
                        )
                        .child(
                            Button::new("apply-global")
                                .outline()
                                .small()
                                .disabled(true)
                                .label(t!("discover.apply_global").to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(t!("discover.apply_m2_note").to_string()),
                        ),
                ),
        );
    }

    pane
}

/// Render the discover page: a search box over the local registry index,
/// and the results list. `hits` is already searched/browsed by the caller
/// (empty query = all entries alphabetically, see `AppState::search`), and
/// stays alive on `RootView::discover_hits` so the virtual list closure can
/// slice it per frame without re-running the search. `installed_names` are
/// the tool names from `mise ls` used to mark already-installed rows.
/// `registry` carries the load state: `Loading` / `Idle` show progress and
/// `Failed` is shown raw (AGENTS.md rule 9).
#[allow(clippy::too_many_arguments)]
pub fn render(
    cx: &mut Context<RootView>,
    search_input: &Entity<InputState>,
    hits: &[SearchHit],
    installed_names: &HashSet<String>,
    scrolling: &VirtualListScrollHandle,
    registry: &Loadable<Vec<ToolEntry>>,
    versions: &Loadable<Vec<RemoteVersion>>,
    versions_tool: Option<&str>,
    not_active_version: Option<String>,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();

    let results: gpui::AnyElement = match registry {
        Loadable::Failed(err) => div()
            .flex_1()
            .min_h_0()
            .child(error_panel(cx, err.as_str()))
            .into_any_element(),
        // `Idle` loads too: mise was only just detected (setup page path).
        Loadable::Idle | Loadable::Loading => div()
            .flex_1()
            .min_h_0()
            .child(centered(&theme, t!("discover.loading").to_string()))
            .into_any_element(),
        // Only reachable when a non-empty query matched nothing: browse mode
        // (empty query) always yields the full registry.
        Loadable::Ready(_) if hits.is_empty() => div()
            .flex_1()
            .min_h_0()
            .child(centered(&theme, t!("discover.no_results").to_string()))
            .into_any_element(),
        Loadable::Ready(_) => {
            let item_sizes = Rc::new(vec![Size::new(px(0.), px(ROW_HEIGHT)); hits.len()]);
            let installed = installed_names.clone();
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
                .child(Scrollbar::vertical(scrolling))
                .into_any_element()
        }
    };

    // Search bar spans the full width; results and the versions pane sit
    // side by side below it.
    div()
        .v_flex()
        .flex_1()
        .min_h_0()
        .h_full()
        .bg(theme.background)
        .child(search_bar(cx, search_input))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .child(results)
                .child(versions_pane(
                    cx,
                    versions_tool,
                    versions,
                    not_active_version,
                )),
        )
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
            // Selecting a tool on the discover page also kicks off the
            // async remote-versions fetch (see `select_discover_tool`).
            this.select_discover_tool(click_name.clone(), cx);
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
