use crate::app::RootView;
use crate::state::Loadable;
use crate::theme::monospace_family;
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
use mise_core::DeepSearchHit;
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
    has_project: bool,
    apply_busy: bool,
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
        // No tool picked yet: the pane's centered body below already says
        // 「选择一个工具查看可用版本」. Rendering it here too would print the
        // same sentence twice (once top-left, once centered), so the title
        // slot stays empty until a tool is actually selected.
        None => String::new(),
    };
    // Keep the title row laid out even when empty: it owns the spacing between
    // the search column and the versions list, and an unrendered title would
    // collapse that gap once a tool is picked.
    pane = pane.child(
        div()
            .text_sm()
            .font_medium()
            .min_h_5()
            .when(!title.is_empty(), |d| d.child(title)),
    );

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
                        // Hairline separator between version rows.
                        .border_b_1()
                        .border_color(theme.border.opacity(0.6))
                        .child(
                            div()
                                .text_sm()
                                .font_family(monospace_family())
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

    // Rule 4: installing and activating are separate actions. After an
    // install the tool is on the machine but NOT active; the apply buttons
    // open the SAME preview + confirm flow as the tool detail pane. The
    // 「应用到项目」 button only renders when a project is actually in
    // context — without one, a button labeled "this project" would silently
    // write the global config (dishonest labeling; AGENTS.md rule 2).
    if let Some(version) = not_active_version {
        let mut actions = div().h_flex().gap_2();
        if let Some(tool) = tool {
            let apply_spec = format!("{tool}@{version}");
            let spec_global = apply_spec.clone();
            actions = actions.child(
                Button::new("apply-global")
                    .outline()
                    .small()
                    .disabled(apply_busy)
                    .label(t!("discover.apply_global").to_string())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.prompt_apply(spec_global.clone(), true, window, cx);
                    })),
            );
            if has_project {
                let spec_project = apply_spec.clone();
                actions = actions.child(
                    Button::new("apply-project")
                        .outline()
                        .small()
                        .disabled(apply_busy)
                        .label(t!("discover.apply_project").to_string())
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.prompt_apply(spec_project.clone(), false, window, cx);
                        })),
                );
            }
        }
        if apply_busy {
            actions = actions.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("tools.apply_running").to_string()),
            );
        }
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
                .child(actions),
        );
    }

    pane
}

/// Which locale key applies when a loaded registry yields zero hits:
/// "the registry itself is empty" (browsing with an empty query) vs "your
/// search matched nothing" (non-empty query). Returns the key, not the
/// translated string, so it is testable without a locale context; the render
/// path wraps it in `t!`.
fn registry_empty_message(query: &str) -> &'static str {
    if query.trim().is_empty() {
        "discover.registry_empty"
    } else {
        "discover.no_results"
    }
}

/// Render the discover page: a search box over the local registry index,
/// and the results list. `hits` is already searched/browsed by the caller
/// (empty query = all entries alphabetically, see `AppState::search`), and
/// stays alive on `RootView::discover_hits` so the virtual list closure can
/// slice it per frame without re-running the search. `installed_names` are
/// the tool names from `mise ls` used to mark already-installed rows.
/// `query` distinguishes "search matched nothing" from a genuinely empty
/// install. `registry` carries the load state: `Loading` / `Idle` show
/// progress and `Failed` is shown raw with a retry (AGENTS.md rule 9).
#[allow(clippy::too_many_arguments)]
pub fn render(
    cx: &mut Context<RootView>,
    search_input: &Entity<InputState>,
    hits: &[SearchHit],
    installed_names: &HashSet<String>,
    scrolling: &VirtualListScrollHandle,
    registry: &Loadable<Vec<ToolEntry>>,
    query: &str,
    versions: &Loadable<Vec<RemoteVersion>>,
    versions_tool: Option<&str>,
    not_active_version: Option<String>,
    has_project: bool,
    apply_busy: bool,
    deep: &Loadable<Vec<DeepSearchHit>>,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();

    let results: gpui::AnyElement = match registry {
        Loadable::Failed(err) => div()
            .flex_1()
            .min_h_0()
            .v_flex()
            .p_3()
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
                    .overflow_y_scrollbar()
                    .child(err.to_string()),
            )
            .child(
                Button::new("registry-retry")
                    .outline()
                    .small()
                    .label(t!("common.retry").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.retry_registry(cx);
                    })),
            )
            .into_any_element(),
        // `Idle` loads too: mise was only just detected (setup page path).
        Loadable::Idle | Loadable::Loading => div()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(Spinner::new().xsmall())
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .text_sm()
                            .child(t!("discover.loading").to_string()),
                    ),
            )
            .into_any_element(),
        // Loaded, zero rows. Browsing (empty query) over a loaded registry
        // that yields nothing means the registry itself is empty; a
        // non-empty query is a search that matched nothing.
        Loadable::Ready(_) if hits.is_empty() => div()
            .flex_1()
            .min_h_0()
            .child(centered(
                &theme,
                t!(registry_empty_message(query)).to_string(),
            ))
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

    // Deep search endpoint: the 「联网搜索结果」 section is appended below the
    // local results so both stay visible (deep search never replaces the
    // local list). It is filled ONLY on the user's explicit click.
    let deep_section = deep_section(cx, deep);

    // Search bar spans the full width; results, the deep-search section and
    // the versions pane sit below it.
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
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .child(results)
                        .child(deep_section),
                )
                .child(versions_pane(
                    cx,
                    versions_tool,
                    versions,
                    not_active_version,
                    has_project,
                    apply_busy,
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
                    // User-triggered online deep search (mise search --all).
                    // It never runs automatically: only this click fires the
                    // background fetch (AGENTS.md rule 5).
                    Button::new("deep-search")
                        .outline()
                        .small()
                        .label(t!("discover.deep_search").to_string())
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.run_deep_search(cx);
                        })),
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
        // Hairline separator between result rows (matches deep_row below).
        .border_b_1()
        .border_color(theme.border.opacity(0.6))
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

/// Online-wide-search section below the local results: label 「联网结果」, a
/// separate loading/empty/failure state (raw error, with retry) and rows
/// with the SAME install action as local rows. It never replaces the local
/// list; deep results install the verbatim (backend-prefixed) name with
/// `latest` through `start_install`.
fn deep_section(
    cx: &mut Context<RootView>,
    deep: &Loadable<Vec<DeepSearchHit>>,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();

    // Hidden until the user triggered a deep search at least once — the
    // local list alone is honest output at that point (no reserved space).
    if deep == &Loadable::Idle {
        return div();
    }

    let mut section = div()
        .v_flex()
        .flex_1()
        .min_h_0()
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.background);

    section = section
        .child(
            div()
                .px_3()
                .pt_2()
                .text_xs()
                .font_medium()
                .text_color(theme.muted_foreground)
                .child(t!("discover.online_results").to_string()),
        )
        .child(match deep {
            Loadable::Idle => div().into_any_element(),
            Loadable::Loading => div()
                .h_flex()
                .px_3()
                .py_2()
                .gap_2()
                .items_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(Spinner::new().xsmall())
                .child(t!("discover.deep_loading").to_string())
                .into_any_element(),
            Loadable::Failed(err) => div()
                .v_flex()
                .px_3()
                .py_2()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(t!("discover.deep_failed").to_string()),
                )
                .child(
                    // Raw error verbatim (AGENTS.md rule 9), never
                    // abstracted into a generic 「失败了」.
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .overflow_y_scrollbar()
                        .child(err.clone()),
                )
                .child(
                    Button::new("deep-search-retry")
                        .outline()
                        .xsmall()
                        .label(t!("common.retry").to_string())
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.retry_deep_search(cx);
                        })),
                )
                .into_any_element(),
            Loadable::Ready(hits) if hits.is_empty() => div()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("discover.deep_empty").to_string())
                .into_any_element(),
            Loadable::Ready(hits) => {
                let mut col = div()
                    .v_flex()
                    .overflow_y_scrollbar()
                    .max_h(px(240.))
                    .gap_0();
                for hit in hits {
                    col = col.child(deep_row(cx, hit).into_any_element());
                }
                col.into_any_element()
            }
        });
    section
}

fn deep_row(cx: &mut Context<RootView>, hit: &DeepSearchHit) -> impl IntoElement {
    let theme = cx.theme();
    let name = hit.name.clone();
    let install_name = name.clone();
    let description = hit.description.clone();

    div()
        .v_flex()
        .px_3()
        .py_2()
        .w_full()
        .min_h_0()
        .overflow_hidden()
        .border_b_1()
        .border_color(theme.border.opacity(0.6))
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .font_family(monospace_family())
                        .min_w_0()
                        .text_ellipsis()
                        .child(name),
                )
                .child(
                    Button::new(SharedString::from(format!("deep-install-{install_name}")))
                        .primary()
                        .xsmall()
                        .label(t!("discover.install_latest").to_string())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            // Verbatim backend-prefixed name; `latest` keeps
                            // this the SAME install path as local results
                            // (`start_install` → `mise install <name>@latest`,
                            // install only — never activate, rule 4).
                            this.run_install(&install_name, "latest", cx);
                        })),
                ),
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

#[cfg(test)]
mod tests {
    use super::registry_empty_message;

    // These tests were written first (the predicate was an inline ternary
    // inside `render` and could not be exercised): running them without the
    // helper was the RED state — `error[E0432]: unresolved import
    // super::registry_empty_message`. The extraction below makes them GREEN.

    /// Pin the exact keys against the locale files so a rename cannot
    /// silently orphan `discover.registry_empty` / `discover.no_results`.
    #[test]
    fn registry_empty_message_keys() {
        assert_eq!(registry_empty_message(""), "discover.registry_empty");
        assert_eq!(registry_empty_message("   "), "discover.registry_empty");
        assert_eq!(registry_empty_message("node"), "discover.no_results");
    }
}
