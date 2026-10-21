//! Tasks page (M4-T2 + M4-T3): the task list (name, description, source,
//! hidden marker, dependency counts), the detail pane (run commands,
//! depends / depends_post / wait_for, the execution-order dependency tree),
//! and the run / watch actions streamed into the bottom terminal panel.
//!
//! Tasks are read + run ONLY: nothing on this page ever writes a config
//! file (rule 3), and mise calls go through `mise-core` exclusively.
//!
//! Like `env.rs`, this module avoids `use gpui_kit::*;` (the kit root
//! re-exports gpui_macros' `test` attribute which shadows std's `#[test]`).

use crate::app::RootView;
use crate::state::Loadable;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::theme::Theme;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, ClickEvent, Context, Entity, SharedString};
use gpui_kit::{div, px};
use mise_core::{Task, TaskGraph};
use rust_i18n::t;

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested at the bottom of this file)
// ---------------------------------------------------------------------------

/// One entry of the execution-order dependency tree of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainNode {
    pub name: String,
    /// The dependency is listed but not defined anywhere in this scope
    /// (`mise` would fail to resolve it) — the UI flags it before running.
    pub missing: bool,
    /// The task is part of a dependency cycle (it can reach itself via
    /// depends / depends_post edges) — mise cannot complete such a chain,
    /// the UI shows a warning instead of a drawable order.
    pub cyclic: bool,
}

/// Locale key for a chain node's annotation. Returns the KEY so tests pin
/// it without a locale context (the render path wraps the same keys in
/// `t!` inline — kept in sync by the tests below).
#[cfg_attr(not(test), allow(dead_code))]
pub fn chain_node_label_key(node: &ChainNode) -> &'static str {
    if node.cyclic {
        "tasks.chain_cycle"
    } else if node.missing {
        "tasks.chain_missing"
    } else {
        ""
    }
}

/// Whether `name` can reach itself again through the graph's dependency
/// edges (both `depends` and `depends_post` count — a post dep that loops
/// back is still a cycle mise cannot finish).
pub fn is_in_cycle(graph: &TaskGraph, name: &str) -> bool {
    let mut adjacency = std::collections::HashMap::new();
    for (t, d) in graph.edges.iter().chain(graph.post_edges.iter()) {
        adjacency
            .entry(t.as_str())
            .or_insert_with(Vec::new)
            .push(d.as_str());
    }
    // DFS from `name`; a return visit to `name` proves a cycle through it.
    let mut stack = vec![name];
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if node == name && !seen.insert(node) {
            return true;
        }
        // The start node itself must not terminate the search.
        if node != name && !seen.insert(node) {
            continue;
        }
        if let Some(deps) = adjacency.get(node) {
            stack.extend(deps.iter().copied());
        }
    }
    false
}

/// The execution-order chain for `task` with per-node classification
/// (missing dependencies, cycle membership), or `None` when the task is
/// itself part of a cycle (then the pane shows the cycle warning instead
/// of an order the runner could never follow to completion).
pub fn execution_chain(graph: Option<&TaskGraph>, tasks: &[Task], task: &str) -> Vec<ChainNode> {
    let Some(graph) = graph else {
        return vec![];
    };
    let names: std::collections::HashSet<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
    mise_core::dependency_order(graph, task)
        .into_iter()
        .map(|name| ChainNode {
            cyclic: is_in_cycle(graph, &name),
            missing: !names.contains(name.as_str()),
            name,
        })
        .collect()
}

/// The filtered list rows: hidden tasks are dropped unless `show_hidden`;
/// a non-blank query matches name, description, or aliases (case
/// insensitive). Blank query + no hidden = the unfiltered, mise-ordered list.
pub fn filter_tasks<'a>(tasks: &'a [Task], query: &str, show_hidden: bool) -> Vec<&'a Task> {
    let q = query.to_lowercase();
    tasks
        .iter()
        .filter(|t| show_hidden || !t.hide)
        .filter(|t| task_matches_filter(t, &q))
        .collect()
}

/// The pure per-task predicate behind `filter_tasks`. It lowercases the
/// query ITSELF (a caller forgetting to do so must not break matching).
pub fn task_matches_filter(task: &Task, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || task.name.to_lowercase().contains(&q)
        || task
            .description
            .as_deref()
            .is_some_and(|d| d.to_lowercase().contains(&q))
        || task.aliases.iter().any(|a| a.to_lowercase().contains(&q))
}

/// Number of DIRECT dependencies that being listed by mise will run in this
/// task's run: `depends` + `depends_post` (wait_for adds NO task to the run
/// — it only delays the start, so it never counts here).
pub fn dep_count(task: &Task) -> usize {
    task.depends.len() + task.depends_post.len()
}

/// How many tasks of the graph actually run this task as one of their
/// dependencies (`depends` or `depends_post` edges pointing at it).
pub fn dependents_count(graph: &TaskGraph, task_name: &str) -> usize {
    graph
        .edges
        .iter()
        .chain(graph.post_edges.iter())
        .filter(|(_, dep)| dep == task_name)
        .count()
}

/// The source display: the raw path when known, otherwise the neutral
/// 「无来源文件」 marker (a task without a recorded source is honest about
/// it rather than showing an empty string).
pub fn source_display(task: &Task) -> Option<String> {
    task.source.as_ref().map(|p| p.display().to_string())
}

// ---------------------------------------------------------------------------
// Render path
// ---------------------------------------------------------------------------

/// Whole page render: task list (filter + show-hidden toggle) on the left,
/// the selected task's detail pane on the right. All view-local state is
/// snapshotted by `RootView::tasks_pane` and passed in owned.
#[allow(clippy::too_many_arguments)]
pub fn render(
    cx: &mut Context<RootView>,
    filter_input: &Entity<InputState>,
    show_hidden: bool,
    tasks: &Loadable<Vec<Task>>,
    graph: Option<&TaskGraph>,
    selected: Option<&str>,
    run_busy: bool,
    watch_running: bool,
) -> AnyElement {
    let theme = cx.theme().clone();
    let filter_text = filter_input.read(cx).value().to_string();

    let mut page = div()
        .flex()
        .flex_row()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .bg(theme.background);

    let list = list_column(
        cx,
        filter_input,
        show_hidden,
        tasks,
        graph,
        &filter_text,
        selected,
    );
    let detail = detail_pane(cx, tasks, graph, selected, run_busy, watch_running);
    page = page.child(list).child(detail);
    page.into_any_element()
}

/// Left column: filter box + 「显示隐藏任务」 toggle + the task rows.
fn list_column(
    cx: &mut Context<RootView>,
    filter_input: &Entity<InputState>,
    show_hidden: bool,
    tasks: &Loadable<Vec<Task>>,
    graph: Option<&TaskGraph>,
    filter_text: &str,
    selected: Option<&str>,
) -> AnyElement {
    let theme = cx.theme().clone();

    let mut col = div()
        .v_flex()
        .w(px(340.0))
        .flex_1()
        .h_full()
        .min_w(px(260.0))
        .max_w(px(420.0))
        .border_r_1()
        .border_color(theme.border)
        .p_2()
        .gap_2();

    col = col.child(
        div()
            .h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(filter_input).cleanable(true).small()),
            )
            .child(
                Button::new("tasks-show-hidden")
                    .outline()
                    .small()
                    .selected(show_hidden)
                    .label(
                        t!(if show_hidden {
                            "tasks.hide_hidden"
                        } else {
                            "tasks.show_hidden"
                        })
                        .to_string(),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.state.tasks_show_hidden = !this.state.tasks_show_hidden;
                        cx.notify();
                    })),
            ),
    );

    let body: AnyElement = match tasks {
        Loadable::Failed(err) => failed_body(cx, err),
        Loadable::Idle | Loadable::Loading => status_body(&theme, t!("tasks.loading").to_string()),
        Loadable::Ready(all) => {
            let rows = filter_tasks(all, filter_text, show_hidden);
            if all.is_empty() {
                status_body(&theme, t!("tasks.empty").to_string())
            } else if rows.is_empty() {
                status_body(&theme, t!("tasks.no_match").to_string())
            } else {
                let mut list = div()
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .gap_1();
                for task in rows {
                    list = list.child(
                        task_row(cx, task, graph, selected.map(String::from)).into_any_element(),
                    );
                }
                list.into_any_element()
            }
        }
        .into_any_element(),
    };
    col = col.child(body);
    col.into_any_element()
}

/* The selected task name is snapshotted once per render by `tasks_pane`
and passed down as a plain argument — reading it off the view entity
inside this render path would re-enter `Entity::read` and panic. */

fn task_row(
    cx: &mut Context<RootView>,
    task: &Task,
    graph: Option<&TaskGraph>,
    selected: Option<String>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let name = task.name.clone();
    let is_selected = selected.as_deref() == Some(name.as_str());
    let dependents = graph.map(|g| dependents_count(g, &name)).unwrap_or(0);
    // The listener takes its OWN clone: `name` must survive for the row text.
    let listener_name = name.clone();

    let row = div()
        .id(SharedString::from(format!("task-row-{name}")))
        .v_flex()
        .px_3()
        .py_2()
        .rounded_md()
        .gap_1()
        .cursor_pointer()
        .border_1()
        .border_color(theme.border.opacity(0.5))
        .when(is_selected, |d| d.bg(theme.muted.opacity(0.5)))
        .when(!is_selected, |d| {
            d.hover(|d| d.bg(theme.muted.opacity(0.3)))
        })
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.state.selected_task = Some(listener_name.clone());
            cx.notify();
        }))
        // Name line with markers.
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .font_family(crate::theme::monospace_family())
                        .min_w_0()
                        .text_ellipsis()
                        .overflow_hidden()
                        .child(name.clone()),
                )
                .when(task.hide, |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("tasks.hidden_marker").to_string()),
                    )
                })
                .when(dep_count(task) > 0, |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme.info)
                            .child(t!("tasks.deps_count", n = dep_count(task)).to_string()),
                    )
                })
                .when(dependents > 0, |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("tasks.dependents_count", n = dependents).to_string()),
                    )
                }),
        )
        .when_some(
            task.description.as_deref().filter(|d| !d.is_empty()),
            |d, desc| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .min_w_0()
                        .text_ellipsis()
                        .overflow_hidden()
                        .child(desc.to_string()),
                )
            },
        )
        .when_some(source_display(task), |d, source| {
            d.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .min_w_0()
                    .child(Icon::new(IconName::File).size_3().into_any_element())
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .overflow_hidden()
                            .font_family(crate::theme::monospace_family())
                            .child(source),
                    ),
            )
        });
    row.into_any_element()
}

/// Right pane: the selected task's detail, or a neutral placeholder.
#[allow(clippy::too_many_arguments)]
fn detail_pane(
    cx: &mut Context<RootView>,
    tasks: &Loadable<Vec<Task>>,
    graph: Option<&TaskGraph>,
    selected: Option<&str>,
    run_busy: bool,
    watch_running: bool,
) -> AnyElement {
    let theme = cx.theme().clone();

    let Some(selected) = selected else {
        return div()
            .flex_1()
            .h_full()
            .items_center()
            .justify_center()
            .text_color(theme.muted_foreground)
            .text_sm()
            .child(t!("tasks.select_task").to_string())
            .into_any_element();
    };
    let Some(all) = tasks.ready() else {
        return status_body(&theme, t!("tasks.loading").to_string());
    };
    let Some(task) = all.iter().find(|t| t.name == selected) else {
        return status_body(&theme, t!("tasks.select_task").to_string());
    };

    let mono = crate::theme::monospace_family();
    let name = task.name.clone();
    // Action listeners and the chain comparison each take their own clones;
    // `name` stays owned by the pane for the run/warn displays below.
    let run_name = name.clone();
    let watch_name = name.clone();

    // Cycle state decides whether the chain section shows an order or the
    // cycle warning: a task inside a cycle has no completable execution
    // order and the Run button stays honest by still offering mise's run
    // (mise decides; the warning explains what will happen).
    let chain = execution_chain(graph, all, &name);
    let any_cyclic = chain.iter().any(|n| n.cyclic);

    let mut col = div()
        .v_flex()
        .flex_1()
        .h_full()
        .min_w_0()
        .overflow_y_scrollbar()
        .p_3()
        .gap_3();

    // Header: name + description + actions.
    col = col.child(
        div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .font_family(mono)
                            .min_w_0()
                            .text_ellipsis()
                            .overflow_hidden()
                            .child(name.clone()),
                    )
                    .when(task.hide, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .px_1p5()
                                .rounded_full()
                                .text_color(theme.muted_foreground)
                                .bg(theme.muted.opacity(0.6))
                                .child(t!("tasks.hidden_marker").to_string()),
                        )
                    }),
            )
            .when_some(
                task.description.as_deref().filter(|d| !d.is_empty()),
                |d, desc| {
                    d.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(desc.to_string()),
                    )
                },
            ),
    );

    // Actions: run + watch; watch carries its live state.
    let watch_label = if watch_running {
        t!("tasks.watching").to_string()
    } else {
        t!("tasks.watch").to_string()
    };
    col = col.child(
        div()
            .h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("task-run")
                    .primary()
                    .small()
                    .icon(gpz_icon_run())
                    .label(if run_busy {
                        t!("tasks.run_running").to_string()
                    } else {
                        t!("tasks.run").to_string()
                    })
                    .disabled(run_busy)
                    .on_click(cx.listener({
                        move |this, _: &ClickEvent, _window, cx| {
                            this.run_task(&run_name, false, cx);
                        }
                    })),
            )
            .child(
                Button::new("task-watch")
                    .outline()
                    .small()
                    .label(watch_label)
                    // A running watch must not be re-entrant: spawning a
                    // second `mise watch` would duplicate the watcher and
                    // let the first one's cleanup erase the state.
                    .disabled(run_busy || watch_running)
                    .on_click(cx.listener({
                        move |this, _: &ClickEvent, _window, cx| {
                            this.run_task(&watch_name, true, cx);
                        }
                    })),
            )
            .when(watch_running, |d| {
                d.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(theme.info)
                        .child(Spinner::new().xsmall())
                        .child(t!("tasks.watching_state").to_string()),
                )
            }),
    );

    // The honest watch note: mise-hub's streaming bridge has NO kill/stop
    // handle for the spawned process, so there is no Stop button — the
    // panel states exactly what stopping a watch actually requires.
    if watch_running {
        col = col.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("tasks.watch_stop_note").to_string()),
        );
    }

    // Run commands.
    if !task.run.is_empty() {
        col = col
            .child(field_label(t!("tasks.run_commands").as_ref()))
            .child(
                div()
                    .v_flex()
                    .gap_0p5()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme.muted.opacity(0.3))
                    .children(task.run.iter().map(|cmd| {
                        div()
                            .text_xs()
                            .font_family(mono)
                            .min_w_0()
                            .text_ellipsis()
                            .overflow_hidden()
                            .child(cmd.clone())
                            .into_any_element()
                    })),
            );
    }

    // Dependencies section: depends / depends_post / wait_for chips.
    col = col
        .child(field_label(t!("tasks.dependencies").as_ref()))
        .child(
            div()
                .v_flex()
                .gap_1()
                .children(dep_section(cx, "tasks.depends", &task.depends))
                .children(dep_section(cx, "tasks.depends_post", &task.depends_post))
                .children(dep_section(cx, "tasks.wait_for", &task.wait_for)),
        );

    // Execution-order chain (or the cycle warning).
    col = col
        .child(field_label(t!("tasks.chain").as_ref()))
        .child(if any_cyclic {
            div()
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(t!("tasks.cycle_warn", task = name.clone()).to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!("tasks.cycle_hint").to_string()),
                )
                .into_any_element()
        } else if chain.is_empty() {
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(t!("tasks.chain_empty").to_string())
                .into_any_element()
        } else {
            div()
                .v_flex()
                .gap_0p5()
                .pl_3()
                .border_l_2()
                .border_color(theme.border)
                .children(chain.iter().map(|node| {
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .child(
                            div()
                                .font_family(mono)
                                .min_w_0()
                                .text_ellipsis()
                                .overflow_hidden()
                                .text_color(if node.missing {
                                    theme.warning
                                } else if node.name == name {
                                    theme.foreground
                                } else {
                                    theme.muted_foreground
                                })
                                .child(if node.name == name {
                                    format!("▶ {}", node.name)
                                } else {
                                    node.name.clone()
                                }),
                        )
                        .when(node.cyclic, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.danger)
                                    .child(t!("tasks.chain_cycle").to_string()),
                            )
                        })
                        .when(node.missing, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.warning)
                                    .child(t!("tasks.chain_missing").to_string()),
                            )
                        })
                        .into_any_element()
                }))
                .into_any_element()
        });

    col.into_any_element()
}

/// One dependency-group section header + its chips. Empty groups render as
/// nothing at all (an empty list must not draw a header pointing at nothing).
fn dep_section(
    cx: &Context<RootView>,
    label_key: &'static str,
    deps: &[String],
) -> Vec<AnyElement> {
    if deps.is_empty() {
        return vec![];
    }
    let theme = cx.theme();
    let mut out = vec![
        div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .pt_1()
            .child(t!(label_key).to_string())
            .into_any_element(),
    ];
    out.push(
        div()
            .h_flex()
            .flex_wrap()
            .gap_1()
            .children(deps.iter().map(|d| {
                div()
                    .text_xs()
                    .px_1p5()
                    .py_0p5()
                    .rounded_md()
                    .bg(theme.muted.opacity(0.5))
                    .font_family(crate::theme::monospace_family())
                    .text_color(theme.foreground)
                    .child(d.clone())
                    .into_any_element()
            }))
            .into_any_element(),
    );
    out
}

/// Run action's play glyph.
fn gpz_icon_run() -> Icon {
    Icon::new(IconName::Play)
}

fn field_label(text: &str) -> AnyElement {
    div()
        .text_xs()
        .font_medium()
        .child(text.to_string())
        .into_any_element()
}

/// Failure state with the raw error verbatim (AGENTS.md rule 9) and retry.
fn failed_body(cx: &Context<RootView>, err: &str) -> AnyElement {
    let theme = cx.theme();
    div()
        .v_flex()
        .gap_2()
        .p_2()
        .child(
            div()
                .text_sm()
                .text_color(theme.danger)
                .child(t!("tasks.failed").to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(err.to_string()),
        )
        .child(
            Button::new("tasks-retry")
                .outline()
                .small()
                .label(t!("common.retry").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.state.tasks = Loadable::Loading;
                    this.load_tasks_for_selection(cx);
                })),
        )
        .into_any_element()
}

fn status_body(theme: &Theme, message: String) -> AnyElement {
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

// Keep the pane height honest: the run/want busy flags above all read live
// session status via `state.task_watch_running` / `in_flight`; no dead keys.

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str, hide: bool) -> Task {
        Task {
            name: name.into(),
            aliases: vec![],
            description: None,
            source: None,
            depends: vec![],
            depends_post: vec![],
            wait_for: vec![],
            hide,
            global: false,
            run: vec![],
            file: None,
        }
    }

    fn graph_of(edges: &[(&'static str, &'static str)]) -> TaskGraph {
        TaskGraph {
            nodes: vec![],
            edges: edges
                .iter()
                .map(|(t, d)| ((*t).into(), (*d).into()))
                .collect(),
            post_edges: vec![],
        }
    }
    #[test]
    fn filter_matches_name_description_and_aliases() {
        let mut t = task("build", false);
        t.description = Some("Build the project".into());
        t.aliases = vec!["b".into()];
        // Name (substring, case-insensitive).
        assert!(task_matches_filter(&t, "uil"));
        assert!(task_matches_filter(&t, "BUILD"));
        // Description.
        assert!(task_matches_filter(&t, "the project"));
        // Alias.
        assert!(task_matches_filter(&t, "b"));
        // Nothing matches.
        assert!(!task_matches_filter(&t, "lint"));
        // Blank query matches everything.
        assert!(task_matches_filter(&t, ""));
    }

    #[test]
    fn hidden_tasks_dropped_unless_show_hidden() {
        let tasks = vec![task("public", false), task("secret", true)];
        let visible = filter_tasks(&tasks, "", false);
        assert_eq!(
            visible.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["public"]
        );
        let with_hidden = filter_tasks(&tasks, "", true);
        assert_eq!(with_hidden.len(), 2);
    }

    #[test]
    fn filter_query_applies_together_with_hidden_rule() {
        let tasks = vec![task("build", false), task("hidden-build", true)];
        let rows = filter_tasks(&tasks, "build", false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "build");
    }

    #[test]
    fn chain_marks_missing_dependencies() {
        // test -> nope (not in tasks): `nope` must be flagged missing, not
        // silently dropped.
        let g = graph_of(&[("test", "nope")]);
        let tasks = vec![task("test", false)];
        let chain = execution_chain(Some(&g), &tasks, "test");
        assert_eq!(chain.len(), 2);
        assert!(
            chain
                .iter()
                .any(|n| n.name == "nope" && n.missing && !n.cyclic)
        );
        assert!(
            chain
                .iter()
                .any(|n| n.name == "test" && !n.missing && !n.cyclic)
        );
    }

    #[test]
    fn chain_marks_cycle_members() {
        // a -> b -> a cycle; running `a`: every member is flagged cyclic.
        let g = graph_of(&[("a", "b"), ("b", "a")]);
        let tasks = vec![task("a", false), task("b", false)];
        let chain = execution_chain(Some(&g), &tasks, "a");
        assert_eq!(chain.len(), 2);
        for node in &chain {
            if node.name == "a" || node.name == "b" {
                assert!(node.cyclic);
            }
        }
        assert_eq!(chain_node_label_key(&chain[0]), "tasks.chain_cycle");
    }

    #[test]
    fn chain_of_clean_task_has_no_markers() {
        // build -> lint, both defined.
        let g = graph_of(&[("build", "lint")]);
        let tasks = vec![task("build", false), task("lint", false)];
        let chain = execution_chain(Some(&g), &tasks, "build");
        assert_eq!(
            chain,
            vec![
                ChainNode {
                    name: "lint".into(),
                    missing: false,
                    cyclic: false,
                },
                ChainNode {
                    name: "build".into(),
                    missing: false,
                    cyclic: false,
                }
            ]
        );
    }

    #[test]
    fn chain_without_graph_is_empty() {
        assert!(execution_chain(None, &[task("x", false)], "x").is_empty());
    }

    #[test]
    fn is_in_cycle_detects_self_and_shared_loops() {
        assert!(is_in_cycle(&graph_of(&[("a", "a")]), "a"));
        let g = graph_of(&[("a", "b"), ("b", "a"), ("c", "a")]);
        assert!(is_in_cycle(&g, "a"));
        assert!(is_in_cycle(&g, "b"));
        // `c` only POINTS at the cycle; it is not a member of one.
        assert!(!is_in_cycle(&g, "c"));
    }

    #[test]
    fn is_in_cycle_false_for_trees() {
        let g = graph_of(&[("test", "build"), ("build", "lint")]);
        assert!(!is_in_cycle(&g, "test"));
        assert!(!is_in_cycle(&g, "lint"));
    }

    #[test]
    fn dep_and_dependents_counts() {
        let mut build = task("build", false);
        build.depends = vec!["lint".into()];
        build.depends_post = vec!["notify".into()];
        build.wait_for = vec!["setup".into()]; // NOT part of the walk
        assert_eq!(dep_count(&build), 2);

        // lint is depended on by build AND test; setup by build only, but
        // wait_for contributes nothing to either count of `setup`.
        let mut test = task("test", false);
        test.depends = vec!["lint".into()];
        let g = mise_core::build_graph(vec![build, test, task("lint", false), {
            let mut setup = task("setup", false);
            setup.wait_for = vec!["noop".into()];
            setup
        }]);
        assert_eq!(dependents_count(&g, "lint"), 2);
        assert_eq!(dependents_count(&g, "noop"), 0);
    }

    #[test]
    fn source_display_uses_path_or_none() {
        let mut t = task("build", false);
        assert_eq!(source_display(&t), None);
        t.source = Some(std::path::PathBuf::from("/proj/mise.toml"));
        assert_eq!(source_display(&t), Some("/proj/mise.toml".to_string()));
    }
}
