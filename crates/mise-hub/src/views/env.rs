//! Environment-variable page (M3-T2 + M3-T3): declarations, effective
//! values, the three-column diff, PATH segmentation sources, and the
//! add / edit / delete flows.
//!
//! Redaction: mise redacts variables declared with `redact = true` as the
//! literal `[redacted]` by DEFAULT (verified on 2026.10.15; `mise set
//! --no-redact` lifts it, and core passes that flag when listing). Hiding
//! secrets is therefore BOTH a data-layer flag (`--no-redact` requested,
//! values re-read exactly) AND a display concern here — values render
//! masked until the user reveals them, and copying is refused while a value
//! stays hidden (the page carries a note stating exactly that).
//!
//! NOTE: this module deliberately avoids `use gpui_kit::*;` — the kit root
//! re-exports gpui_macros' own `test` attribute which, combined with the
//! `#[test]` fns here, shadows std's `#[test]` and recurses forever
//! (observed as a rustc SIGBUS). All gpui items are imported explicitly.

use crate::app::RootView;
use crate::state::{EnvTab, Loadable, resolved_env_name};
use crate::theme::monospace_family;
use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable as _;
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
use gpui_kit::{AnyElement, App, ClickEvent, Context, Entity, SharedString, WeakEntity};
use gpui_kit::{div, px};
use mise_core::{EnvDeclaration, EnvVar, is_redacted, path_segments};
use rust_i18n::t;
use std::collections::HashSet;
use std::path::Path;

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested at the bottom of this file)
// ---------------------------------------------------------------------------

/// How one PATH segment earned its place, as the page annotates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegmentKind {
    /// A bin dir under the installed-files tree of the named tool.
    ToolBin(String),
    /// Provided through a config declaration (project or global).
    Config,
    /// Neither of the above — system / inherited.
    System,
}

/// The pure segment classifier. Precedence: tool bin dir (segment contains
/// `mise/installs/<tool>/`) wins over config membership, then system.
pub fn classify_path_segment(segment: &str, config_tokens: &HashSet<String>) -> PathSegmentKind {
    if let Some(rest) = segment.split("mise/installs/").nth(1) {
        let tool = rest.split('/').find(|c| !c.is_empty()).unwrap_or_default();
        if !tool.is_empty() {
            return PathSegmentKind::ToolBin(tool.to_string());
        }
    }
    if config_tokens.contains(segment) {
        return PathSegmentKind::Config;
    }
    PathSegmentKind::System
}

/// Path-shaped tokens config declarations provide: every separator-separated
/// piece of every declared value (e.g. `PATH=/proj/bin:$HOME/bin`) and whole
/// path-shaped values (`JAVA_HOME=/opt/jdk`). The segment classifier matches
/// against this set, so a dir the user declared counts as config-provided.
pub fn config_path_tokens(declarations: &[EnvDeclaration]) -> HashSet<String> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    declarations
        .iter()
        .flat_map(|d| d.value.split(sep))
        .map(|tok| tok.trim().to_string())
        .filter(|tok| !tok.is_empty())
        .collect()
}

/// Locale key for a PATH segment's source hint; returns the KEY so tests pin
/// it without a locale context (the render path wraps it in `t!`).
pub fn path_kind_label_key(kind: &PathSegmentKind) -> &'static str {
    match kind {
        PathSegmentKind::ToolBin(_) => "env.path_tool",
        PathSegmentKind::Config => "env.path_config",
        PathSegmentKind::System => "env.path_system",
    }
}

/// Segment + classified kind for one entry of the effective PATH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentHint {
    pub segment: String,
    pub kind: PathSegmentKind,
}

/// Annotate every segment of a PATH-like value.
pub fn path_segment_hints(value: &str, config_tokens: &HashSet<String>) -> Vec<SegmentHint> {
    path_segments(value)
        .into_iter()
        .map(|segment| {
            let kind = classify_path_segment(&segment, config_tokens);
            SegmentHint { segment, kind }
        })
        .collect()
}

/// The variable name `mise set` accepts: non-empty, `[A-Za-z_][A-Za-z0-9_]*`.
/// Empty names and names with `-`, `.`, spaces or leading digits are refused
/// BEFORE any subprocess runs (the inline error shows, nothing is spawned).
pub fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The exact command the confirm dialog previews (AGENTS.md rule 10). Mirrors
/// the argv `Mise::set_env_stream` builds: `mise -C <dir> set NAME=VALUE` for
/// project scope, `mise set --global NAME=VALUE` for global scope. The
/// `NAME=VALUE` pair is quoted when the value contains a space so the
/// displayed command stays paste-able.
pub fn set_command_preview(
    name: &str,
    value: &str,
    global: bool,
    root: Option<&str>,
    file: Option<&str>,
) -> String {
    let pair = format!("{name}={value}");
    let pair_display = if pair.contains(' ') {
        format!("'{}'", pair.replace('\'', "'\\''"))
    } else {
        pair
    };
    if let Some(file) = file {
        // Writing an EXISTING declaration: the declaration's own file is
        // targeted with `--file` — exactly the command `run_set_env` runs.
        match root {
            Some(dir) => format!("mise -C {dir} set --file {file} {pair_display}"),
            None => format!("mise set --file {file} {pair_display}"),
        }
    } else if global {
        format!("mise set --global {pair_display}")
    } else {
        format!(
            "mise -C {} set {pair_display}",
            root.unwrap_or("<missing project>")
        )
    }
}

/// Where a delete (and, similarly, an edit of an unscoped declaration)
/// targets: the row's OWN source file when it is known (`--file` against
/// exactly that file), otherwise the page's scope. A global-sourced row on
/// a project page MUST resolve to its file, never to the page scope —
/// `mise -C <project> unset NAME` would exit 0 while touching nothing
/// (silent no-op, blocker 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsetTarget {
    File(String),
    Scope { global: bool },
}

/// The pure resolution. `page_global` is the CURRENT page's scope (true on a
/// global page); it applies only in the source-unknown fallback.
pub fn unset_target(source: Option<&str>, page_global: bool) -> UnsetTarget {
    match source.map(str::trim).filter(|f| !f.is_empty()) {
        Some(file) => UnsetTarget::File(file.to_string()),
        None => UnsetTarget::Scope {
            global: page_global,
        },
    }
}

/// Declarations for the diff's PROJECT column: ONLY rows whose source lives
/// inside the project root. The current-scope listing also contains GLOBAL
/// declarations (fed by `mise set` in project scope); passing them through
/// unfiltered made global-only variables LOOK like project declarations and
/// silently never be flagged (blocker 4).
pub fn project_scope_declarations(
    declarations: &[EnvDeclaration],
    root: &Path,
) -> Vec<EnvDeclaration> {
    declarations
        .iter()
        .filter(|d| d.source.starts_with(root))
        .cloned()
        .collect()
}

/// Declarations grouped by their source file, sources sorted, rows keeping
/// their input order inside each group.
pub fn group_by_source(declarations: &[EnvDeclaration]) -> Vec<(String, Vec<EnvDeclaration>)> {
    let mut sources: Vec<String> = declarations
        .iter()
        .map(|d| d.source.to_string_lossy().into_owned())
        .collect();
    sources.sort();
    sources.dedup();
    sources
        .into_iter()
        .map(|source| {
            let rows = declarations
                .iter()
                .filter(|d| d.source.to_string_lossy() == source)
                .cloned()
                .collect();
            (source, rows)
        })
        .collect()
}

/// One row of the three-column diff. `None` means "absent in this column".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    pub name: String,
    pub global: Option<String>,
    pub project: Option<String>,
    pub effective: Option<String>,
    pub differ: bool,
}

fn decl_pairs(rows: &[EnvDeclaration]) -> Vec<(String, String)> {
    rows.iter()
        .map(|d| (d.name.clone(), d.value.clone()))
        .collect()
}

/// Declarations for diff comparison with `[redacted]` placeholder rows
/// dropped: mise's redact placeholder is never a real value — treating it
/// as data produced false conflicts (both columns looking unequal, or the
/// effective value "differing" from a value that was never shown).
fn diffable_declarations(rows: Option<&[EnvDeclaration]>) -> Option<Vec<EnvDeclaration>> {
    rows.map(|rows| {
        rows.iter()
            .filter(|d| !is_redacted(&d.value))
            .cloned()
            .collect()
    })
}

fn var_pairs(rows: &[EnvVar]) -> Vec<(String, String)> {
    rows.iter()
        .map(|v| (v.name.clone(), v.value.clone()))
        .collect()
}

fn lookup_pair(pairs: Option<Vec<(String, String)>>, name: &str) -> Option<String> {
    pairs.and_then(|pairs| pairs.into_iter().find(|(n, _)| n == name).map(|(_, v)| v))
}

/// Build the diff table. `global` / `project` are `None` when that column
/// has NO data at all (e.g. the neutral "select a project" column of a
/// global page) — such a column never contributes to `differ`. When both
/// declared columns have data, a row differs when the two declarations
/// (absence included) are not equal; the effective value is additionally
/// flagged when it conflicts with the value declared for this name.
pub fn compute_diff(
    global: Option<&[EnvDeclaration]>,
    project: Option<&[EnvDeclaration]>,
    effective: &[EnvVar],
) -> Vec<DiffRow> {
    // `[redacted]` rows are placeholders, not data (see `diffable_declarations`).
    let global = diffable_declarations(global);
    let project = diffable_declarations(project);
    let mut names: Vec<String> = effective.iter().map(|v| v.name.clone()).collect();
    for rows in [global.as_deref(), project.as_deref()]
        .into_iter()
        .flatten()
    {
        names.extend(decl_pairs(rows).into_iter().map(|(n, _)| n));
    }
    names.sort();
    names.dedup();

    let effective_pairs = var_pairs(effective);

    names
        .into_iter()
        .map(|name| {
            let val_global = lookup_pair(global.as_deref().map(decl_pairs), &name);
            let val_project = lookup_pair(project.as_deref().map(decl_pairs), &name);
            let eff = lookup_pair(Some(effective_pairs.clone()), &name);
            // Both declared columns have data: any inequality between the
            // two declarations (absence included) differs.
            let decl_conflict = global.is_some() && project.is_some() && val_global != val_project;
            // Effective flags a real CONFLICT only: a declared value for the
            // name whose exact value differs from (or vanished from) the
            // runtime environment. Names with no declaration anywhere
            // (system vars) are never flagged just for existing at runtime.
            let eff_conflict = match (&eff, val_global.as_ref(), val_project.as_ref()) {
                (Some(eff_value), Some(decl), _) | (Some(eff_value), _, Some(decl)) => {
                    eff_value != decl
                }
                // Declared somewhere but absent at runtime (stale).
                (None, Some(_), _) | (None, _, Some(_)) => true,
                _ => false,
            };
            DiffRow {
                name,
                global: val_global,
                project: val_project,
                effective: eff,
                differ: decl_conflict || eff_conflict,
            }
        })
        .collect()
}

/// Whether `name`'s value is currently VISIBLE: the global 「显示全部」 toggle
/// reveals everything; otherwise only names revealed one-by-one. The DEFAULT
/// (empty set, show_all off) is hidden — that default is the sensitive-value
/// contract of the whole page.
pub fn is_revealed(show_all: bool, revealed: &HashSet<String>, name: &str) -> bool {
    show_all || revealed.contains(name)
}

/// Copying is allowed only for a value the user can SEE; a hidden value must
/// never land on the clipboard silently. Pure so it is pinned by a test.
pub fn copy_allowed(show_all: bool, revealed: &HashSet<String>, name: &str) -> bool {
    is_revealed(show_all, revealed, name)
}

/// Mask shown instead of a hidden value.
fn display_value(value: &str, revealed: bool) -> String {
    if revealed {
        value.to_string()
    } else {
        "••••••".to_string()
    }
}

/// What the 「编辑变量」 dialog's value input should be prefilled with.
/// A parsed declaration value is only trustworthy when the row's layout
/// was recovered with certainty AND the value is not mise's `[redacted]`
/// placeholder; otherwise the EXACT re-read result is used, and when no
/// exact value is available the edit is refused (confirming the parsed
/// value would write the mangled / placeholder text back into the config).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prefill {
    Declared(String),
    Exact(String),
    Refuse,
}

/// The pure decision. `exact` is the current scope's exact `mise set NAME`
/// result when it is Ready (the literal placeholder still refuses).
pub fn edit_prefill(exact: Option<&str>, decl: &EnvDeclaration) -> Prefill {
    let usable = |v: &str| !decl.ambiguous && !is_redacted(v);
    if usable(&decl.value) {
        return Prefill::Declared(decl.value.clone());
    }
    match exact.filter(|v| !is_redacted(v)) {
        Some(v) => Prefill::Exact(v.to_string()),
        None => Prefill::Refuse,
    }
}

// ---------------------------------------------------------------------------
// Render path (no logic beyond i18n + layout)
// ---------------------------------------------------------------------------

/// Whole page render: header (tabs + 「新增变量」), the body of the selected
/// tab, and a footer note on the display-time-only redaction. View-local
/// state (tabs, reveal / expansion sets, filter text) is passed in as
/// owned snapshots by `RootView::env_pane` so every builder below only
/// reads its arguments — no re-entrant entity reads mid-layout.
#[allow(clippy::too_many_arguments)]
pub fn render(
    cx: &mut Context<RootView>,
    tab: EnvTab,
    show_all: bool,
    filter_input: &Entity<InputState>,
    effective: &Loadable<Vec<EnvVar>>,
    declarations: &Loadable<Vec<EnvDeclaration>>,
    other_declarations: &Loadable<Vec<EnvDeclaration>>,
    exact: &std::collections::HashMap<String, Loadable<String>>,
    revealed: &HashSet<String>,
    expanded: &HashSet<String>,
    filter_text: &str,
    // Snapshot by `env_pane` (which holds `&mut self`): reading these off
    // the view entity here would re-enter `Entity::read` mid-layout and
    // panic — the entity is leased out while this very render runs.
    project_in_context: bool,
    project_root: Option<String>,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();
    let filter = filter_text.to_lowercase();

    let mut col = div()
        .v_flex()
        .flex_1()
        .h_full()
        .min_w_0()
        .p_3()
        .gap_2()
        .bg(theme.background);

    // Header: three tabs + add action.
    let tabs = [
        (
            EnvTab::Declarations,
            "env-tab-declarations",
            "env.tab_declarations",
        ),
        (EnvTab::Effective, "env-tab-effective", "env.tab_effective"),
        (EnvTab::Diff, "env-tab-diff", "env.tab_diff"),
    ];
    let mut header = div().h_flex().gap_2().items_center();
    for (tab_kind, id, key) in tabs {
        header = header.child(
            Button::new(id)
                .outline()
                .small()
                .selected(tab == tab_kind)
                .label(t!(key).to_string())
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.env_tab = tab_kind;
                    cx.notify();
                })),
        );
    }
    col = col.child(
        div()
            .h_flex()
            .justify_between()
            .gap_2()
            .items_center()
            .child(header)
            .child(
                Button::new("env-add")
                    .primary()
                    .small()
                    .icon(Icon::new(IconName::Plus))
                    .label(t!("env.add").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.prompt_env_edit(None, window, cx);
                    })),
            ),
    );

    // The diff compares GLOBAL listings against the CURRENT scope's: on a
    // project page that is (global=other listing, project=current listing);
    // on a global page the project column has no data at all and the diff
    // shows the neutral 「选择项目」 state instead.
    let global_col: Option<Vec<EnvDeclaration>> = if project_in_context {
        other_declarations.ready().cloned()
    } else {
        declarations.ready().cloned()
    };
    let project_col: Option<Vec<EnvDeclaration>> = if project_in_context {
        // Blocker 4: the current-scope listing INCLUDES global declarations;
        // only rows sourced INSIDE the project root count as project rows.
        declarations.ready().map(|rows| {
            let root = project_root.clone().unwrap_or_default();
            project_scope_declarations(rows, Path::new(&root))
        })
    } else {
        None
    };

    // Config-provided PATH tokens: declarations of BOTH scopes feed the
    // segment classifier (passed on into the PATH section below).
    let mut config_tokens = HashSet::new();
    for rows in [
        declarations.ready().cloned(),
        other_declarations.ready().cloned(),
    ]
    .into_iter()
    .flatten()
    {
        config_tokens.extend(config_path_tokens(&rows));
    }

    let body: AnyElement = match tab {
        EnvTab::Declarations => declarations_tab(
            cx,
            declarations,
            effective,
            other_declarations,
            exact,
            show_all,
            revealed,
            expanded,
        )
        .into_any_element(),
        EnvTab::Effective => effective_tab(
            cx,
            effective,
            filter_input,
            show_all,
            revealed,
            &filter,
            &config_tokens,
        )
        .into_any_element(),
        EnvTab::Diff => {
            diff_tab(cx, global_col, project_col, effective, project_in_context).into_any_element()
        }
    };
    col = col.child(body);

    // Sensitive-value policy note: mise redacts by default, the app
    // re-reads values with --no-redact, and this page keeps the values
    // masked — the note states exactly that.
    col = col.child(
        div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(t!("env.redact_note").to_string()),
    );

    col
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

/// Failure state with the raw error verbatim (AGENTS.md rule 9) and a retry.
fn failed_body(cx: &Context<RootView>, err: &str) -> AnyElement {
    let theme = cx.theme();
    div()
        .flex_1()
        .min_h_0()
        .v_flex()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(theme.danger)
                .child(t!("env.failed").to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(err.to_string()),
        )
        .child(
            Button::new("env-retry")
                .outline()
                .small()
                .label(t!("common.retry").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.state.env_declarations = Loadable::Loading;
                    this.state.env_effective = Loadable::Loading;
                    this.load_env_for_selection(false, cx);
                })),
        )
        .into_any_element()
}

/// 「声明」 tab: declarations of the current scope grouped by source file.
/// Rows whose name is also effective are highlighted (declared AND active,
/// spec §6.5); a revealed row expands its override chain (other scope's
/// declaration + the effective value) — the two files we can see.
#[allow(clippy::too_many_arguments)]
fn declarations_tab(
    cx: &mut Context<RootView>,
    declarations: &Loadable<Vec<EnvDeclaration>>,
    effective: &Loadable<Vec<EnvVar>>,
    other_declarations: &Loadable<Vec<EnvDeclaration>>,
    exact: &std::collections::HashMap<String, Loadable<String>>,
    show_all: bool,
    revealed: &HashSet<String>,
    expanded: &HashSet<String>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let effective_names: HashSet<String> = effective
        .ready()
        .map(|rows| rows.iter().map(|v| v.name.clone()).collect())
        .unwrap_or_default();

    match declarations {
        Loadable::Failed(err) => failed_body(cx, err),
        Loadable::Idle | Loadable::Loading => status_body(&theme, t!("env.loading").to_string()),
        Loadable::Ready(rows) if rows.is_empty() => {
            status_body(&theme, t!("env.declarations_empty").to_string())
        }
        Loadable::Ready(rows) => {
            let mut list = div()
                .v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .gap_1();
            for (source, rows) in group_by_source(rows) {
                list = list.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
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
                                .font_family(monospace_family())
                                .child(source),
                        )
                        .into_any_element(),
                );
                for decl in rows {
                    // The override chain's two extra values: the OTHER scope's
                    // declaration of the name plus the effective value —
                    // snapshotted from the passed-in loadables (no re-entrant
                    // entity reads on the render path).
                    let other = other_declarations
                        .ready()
                        .and_then(|list| list.iter().find(|d| d.name == decl.name).cloned());
                    let eff_value = effective.ready().and_then(|list| {
                        list.iter()
                            .find(|v| v.name == decl.name)
                            .map(|v| v.value.clone())
                    });
                    list = list.child(
                        declaration_row(
                            cx,
                            &decl,
                            &effective_names,
                            other,
                            eff_value,
                            exact,
                            show_all,
                            revealed,
                            expanded.contains(&decl.name),
                        )
                        .into_any_element(),
                    );
                }
            }
            list.into_any_element()
        }
        .into_any_element(),
    }
}

/// One declaration row: name, masked value (+ reveal / copy / edit / delete),
/// the 「已生效」 marker, and (when expanded) the override chain.
#[allow(clippy::too_many_arguments)]
fn declaration_row(
    cx: &mut Context<RootView>,
    decl: &EnvDeclaration,
    effective_names: &HashSet<String>,
    other: Option<EnvDeclaration>,
    eff_value: Option<String>,
    exact: &std::collections::HashMap<String, Loadable<String>>,
    show_all: bool,
    revealed: &HashSet<String>,
    expanded: bool,
) -> AnyElement {
    let theme = cx.theme().clone();
    let mono = monospace_family();
    let name = decl.name.clone();
    // The displayed value: an ambiguous row prefers the exact re-read once
    // it has landed; until then the parsed value renders (honestly marked).
    let raw_value = exact
        .get(&name)
        .and_then(|s| s.ready().cloned())
        .unwrap_or_else(|| decl.value.clone());
    // mise's `[redacted]` placeholder is never a real value: it stays
    // masked even when the row (or the whole page) is "revealed".
    let revealed = is_revealed(show_all, revealed, &name) && !is_redacted(&raw_value);
    let exact_state = exact.get(&name);
    let also_effective = effective_names.contains(&name);

    let mut row = div()
        .v_flex()
        .px_3()
        .py_2()
        .rounded_md()
        .gap_1()
        .border_1()
        .border_color(theme.border.opacity(0.5))
        .when(also_effective, |d| d.bg(theme.info.opacity(0.08)));

    // Name line: click toggles the override chain expansion.
    row = row.child(
        div()
            .id(SharedString::from(format!("decl-{}", name)))
            .h_flex()
            .items_center()
            .gap_2()
            .flex_1()
            .min_w_0()
            .cursor_pointer()
            .on_click(cx.listener({
                let name = name.clone();
                move |this, _: &ClickEvent, _window, cx| {
                    this.toggle_env_expanded(&name, cx);
                }
            }))
            .child(
                div()
                    .text_sm()
                    .font_medium()
                    .font_family(mono)
                    .min_w_0()
                    .text_ellipsis()
                    .overflow_hidden()
                    .child(name.clone()),
            )
            .when(decl.ambiguous, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(t!("env.ambiguous").to_string()),
                )
            })
            .when(also_effective, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme.info)
                        .child(t!("env.also_effective").to_string()),
                )
            })
            .into_any_element(),
    );

    // Value line: hidden by default; the exact re-read state replaces the
    // parsed value once it is ready (spinner while reading, raw error on
    // failure — rule 9).
    let mut value_line = div().h_flex().items_center().gap_2().min_w_0().flex_1();
    match exact_state {
        Some(Loadable::Loading) => {
            value_line = value_line.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Spinner::new().xsmall())
                    .child(t!("env.exact_loading").to_string())
                    .into_any_element(),
            );
        }
        Some(Loadable::Failed(err)) => {
            value_line = value_line.child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .min_w_0()
                    .text_ellipsis()
                    .overflow_hidden()
                    .child(err.clone())
                    .into_any_element(),
            );
        }
        _ => {
            value_line = value_line.child(
                div()
                    .text_sm()
                    .font_family(mono)
                    .min_w_0()
                    .text_ellipsis()
                    .overflow_hidden()
                    .child(display_value(&raw_value, revealed))
                    .into_any_element(),
            );
        }
    }
    let raw = raw_value.clone();
    let copy_name = name.clone();
    let edit_name = name.clone();
    let delete_name = name.clone();
    let delete_source = decl.source.display().to_string();
    value_line = value_line.child(
        div()
            .h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new(SharedString::from(format!("decl-reveal-{}", name)))
                    .outline()
                    .xsmall()
                    .icon(Icon::new(if revealed {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    }))
                    .on_click(cx.listener({
                        let name = name.clone();
                        move |this, _: &ClickEvent, _window, cx| {
                            this.toggle_env_reveal(&name, cx);
                        }
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("decl-copy-{}", name)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Copy))
                    .tooltip(t!("env.copy").to_string())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.copy_env_value(&copy_name, &raw, show_all, cx);
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("decl-edit-{}", name)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Pencil))
                    .tooltip(t!("env.edit").to_string())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.prompt_env_edit(Some(edit_name.clone()), window, cx);
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("decl-delete-{}", name)))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Trash))
                    .tooltip(t!("env.delete").to_string())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.prompt_env_delete(
                            delete_name.clone(),
                            Some(delete_source.clone()),
                            window,
                            cx,
                        );
                    })),
            )
            .into_any_element(),
    );
    row = row.child(value_line);

    // Override chain (expanded): the other scope's declaration of the same
    // name plus the effective value.
    if expanded {
        let mut chain = div()
            .v_flex()
            .gap_0p5()
            .pl_3()
            .border_l_2()
            .border_color(theme.border);
        chain = chain.when_some(other, |col, decl| {
            col.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .font_family(mono)
                    .min_w_0()
                    .text_ellipsis()
                    .overflow_hidden()
                    .child(format!(
                        "{}: {}",
                        t!("env.chain_other"),
                        display_value(&decl.value, revealed)
                    )),
            )
        });
        chain = chain.when_some(eff_value, |col, value| {
            col.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .font_family(mono)
                    .min_w_0()
                    .text_ellipsis()
                    .overflow_hidden()
                    .child(format!(
                        "{}: {}",
                        t!("env.chain_effective"),
                        display_value(&value, revealed)
                    )),
            )
        });
        row = row.child(chain.into_any_element());
    }

    row.into_any_element()
}

/// 「生效」 tab: final values sorted by name (core already sorts), with the
/// filter box, the global 显示全部 toggle, source/tool annotation, copy, and
/// delete (only for plain config declarations). Below the list: the
/// dedicated PATH segmentation view.
fn effective_tab(
    cx: &mut Context<RootView>,
    effective: &Loadable<Vec<EnvVar>>,
    filter_input: &Entity<InputState>,
    show_all: bool,
    revealed: &HashSet<String>,
    filter: &str,
    config_tokens: &HashSet<String>,
) -> AnyElement {
    let theme = cx.theme().clone();

    let mut col = div().v_flex().flex_1().min_h_0().gap_2();

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
                Button::new("env-show-all")
                    .outline()
                    .small()
                    .selected(show_all)
                    .label(
                        t!(if show_all {
                            "env.hide_all"
                        } else {
                            "env.show_all"
                        })
                        .to_string(),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.env_show_all = !this.env_show_all;
                        cx.notify();
                    })),
            )
            .into_any_element(),
    );

    let body: AnyElement = match effective {
        Loadable::Failed(err) => failed_body(cx, err),
        Loadable::Idle | Loadable::Loading => status_body(&theme, t!("env.loading").to_string()),
        Loadable::Ready(rows) => {
            let rows: Vec<EnvVar> = rows
                .iter()
                .filter(|v| filter.is_empty() || v.name.to_lowercase().contains(filter))
                .cloned()
                .collect();
            if rows.is_empty() {
                status_body(
                    &theme,
                    t!(if filter.is_empty() {
                        "env.effective_empty"
                    } else {
                        "env.no_match"
                    })
                    .to_string(),
                )
            } else {
                let mut list = div()
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .gap_1();
                for var in rows {
                    list =
                        list.child(effective_row(cx, &var, show_all, revealed).into_any_element());
                }
                list.into_any_element()
            }
        }
        .into_any_element(),
    };

    col = col.child(body);

    // Dedicated PATH view (annotation of each segment's origin). Only the
    // effective tab carries it — the segments exist only in the runtime value.
    if let Some(path_value) = effective.ready().and_then(|rows| {
        rows.iter()
            .find(|v| v.name == "PATH")
            .map(|v| v.value.clone())
    }) {
        col = col.child(path_section(cx, &path_value, config_tokens));
    }

    col.into_any_element()
}

/// One effective row: name, masked value, source label (config file / tool /
/// system), reveal, copy, and delete only when the row is a plain config
/// declaration (declared in a file, not tool-injected — deleting a tool- or
/// system-injected variable through `mise unset` would silently do nothing).
fn effective_row(
    cx: &mut Context<RootView>,
    var: &EnvVar,
    show_all: bool,
    revealed: &HashSet<String>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let mono = monospace_family();
    let name = var.name.clone();
    let revealed = is_revealed(show_all, revealed, &name);
    let declared = var.source.is_some() && var.tool.is_none();

    let source_label = if let Some(tool) = &var.tool {
        t!("env.tool_injected", tool = tool.clone()).to_string()
    } else if let Some(source) = &var.source {
        source.display().to_string()
    } else {
        t!("env.system").to_string()
    };

    let raw = var.value.clone();
    let copy_name = name.clone();
    let delete_name = name.clone();
    let delete_source = var
        .source
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    div()
        .h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        // Hairline separator between rows so adjacent variables never read
        // as one continuous block.
        .border_b_1()
        .border_color(theme.border.opacity(0.6))
        .rounded_md()
        .hover(|d| d.bg(theme.muted.opacity(0.4)))
        .child(
            div()
                .text_sm()
                .font_medium()
                .font_family(mono)
                .min_w(px(112.))
                .text_ellipsis()
                .overflow_hidden()
                .child(name.clone())
                .into_any_element(),
        )
        .child(
            div()
                .text_sm()
                .font_family(mono)
                .flex_1()
                .min_w_0()
                .text_ellipsis()
                .overflow_hidden()
                .child(display_value(&var.value, revealed))
                .into_any_element(),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .max_w(px(220.0))
                .min_w_0()
                .text_ellipsis()
                .overflow_hidden()
                .child(source_label)
                .into_any_element(),
        )
        .child(
            div()
                .h_flex()
                .gap_1()
                .items_center()
                .child(
                    Button::new(SharedString::from(format!("eff-reveal-{}", name)))
                        .outline()
                        .xsmall()
                        .icon(Icon::new(if revealed {
                            IconName::EyeOff
                        } else {
                            IconName::Eye
                        }))
                        .on_click(cx.listener({
                            let name = name.clone();
                            move |this, _: &ClickEvent, _window, cx| {
                                this.toggle_env_reveal(&name, cx);
                            }
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("eff-copy-{}", name)))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Copy))
                        .tooltip(t!("env.copy").to_string())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.copy_env_value(&copy_name, &raw, show_all, cx);
                        })),
                )
                .when(declared, |d| {
                    d.child(
                        Button::new(SharedString::from(format!("eff-delete-{}", name)))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Trash))
                            .tooltip(t!("env.delete").to_string())
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.prompt_env_delete(
                                    delete_name.clone(),
                                    Some(delete_source.clone()),
                                    window,
                                    cx,
                                );
                            })),
                    )
                })
                .into_any_element(),
        )
        .into_any_element()
}

/// The dedicated PATH view: one row per segment with a source hint. Config
/// tokens (computed in `render` from both scopes' declarations) feed the
/// segment classifier.
fn path_section(
    cx: &mut Context<RootView>,
    path_value: &str,
    config_tokens: &HashSet<String>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let mono = monospace_family();

    let hints = path_segment_hints(path_value, config_tokens);

    let mut section = div()
        .v_flex()
        .gap_1()
        .border_t_1()
        .border_color(theme.border)
        .pt_2();
    section = section.child(
        div()
            .text_xs()
            .font_medium()
            .text_color(theme.muted_foreground)
            .child(t!("env.path_section").to_string()),
    );
    if hints.is_empty() {
        section = section.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("env.path_empty").to_string())
                .into_any_element(),
        );
    }
    for (ix, hint) in hints.iter().enumerate() {
        let label_key = path_kind_label_key(&hint.kind);
        let label = match &hint.kind {
            PathSegmentKind::ToolBin(tool) => t!(label_key, tool = tool.clone()).to_string(),
            _ => t!(label_key).to_string(),
        };
        let color = match hint.kind {
            PathSegmentKind::ToolBin(_) => theme.info,
            PathSegmentKind::Config => theme.success,
            PathSegmentKind::System => theme.muted_foreground,
        };
        section = section.child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_sm()
                .hover(|d| d.bg(theme.muted.opacity(0.3)))
                .child(
                    div()
                        .text_xs()
                        .w_6()
                        .text_color(theme.muted_foreground)
                        .child((ix + 1).to_string())
                        .into_any_element(),
                )
                .child(
                    div()
                        .text_sm()
                        .font_family(mono)
                        .flex_1()
                        .min_w_0()
                        .text_ellipsis()
                        .overflow_hidden()
                        .child(hint.segment.clone())
                        .into_any_element(),
                )
                .child(
                    div()
                        .text_xs()
                        .min_w_0()
                        .text_ellipsis()
                        .overflow_hidden()
                        .text_color(color)
                        .child(label)
                        .into_any_element(),
                ),
        );
    }
    section.into_any_element()
}

/// 「差异」 tab: three columns 全局 / 项目 / 生效, one row per variable name,
/// differing rows highlighted. A `None` project column (global page) renders
/// the neutral 「选择项目」 cell instead of a value, with `project_in_context`
/// false — the column itself carries no data there.
fn diff_tab(
    cx: &mut Context<RootView>,
    global: Option<Vec<EnvDeclaration>>,
    project: Option<Vec<EnvDeclaration>>,
    effective: &Loadable<Vec<EnvVar>>,
    project_in_context: bool,
) -> AnyElement {
    let theme = cx.theme().clone();

    let Some(effective_rows) = effective.ready().cloned() else {
        return status_body(&theme, t!("env.loading").to_string());
    };
    let rows = compute_diff(global.as_deref(), project.as_deref(), &effective_rows);
    if rows.is_empty() {
        return status_body(&theme, t!("env.diff_empty").to_string());
    }

    let mono = monospace_family();
    let mut col = div().v_flex().flex_1().min_h_0().gap_1();
    // Header row.
    col = col.child(
        div()
            .h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .child(cell(
                t!("env.diff_col_global").to_string(),
                mono,
                theme.muted_foreground,
                true,
            ))
            .child(cell(
                t!("env.diff_col_project").to_string(),
                mono,
                theme.muted_foreground,
                true,
            ))
            .child(cell(
                t!("env.diff_col_effective").to_string(),
                mono,
                theme.muted_foreground,
                true,
            )),
    );
    for row in rows {
        col = col.child(diff_row_element(cx, row, project_in_context));
    }
    if !project_in_context {
        col = col.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("env.diff_select_project").to_string()),
        );
    }
    col.into_any_element()
}

/// One diff row: name + the three values (or 「—」 / the neutral project
/// cell), differing rows highlighted with the warning tint.
fn diff_row_element(cx: &Context<RootView>, row: DiffRow, project_present: bool) -> AnyElement {
    let theme = cx.theme();
    let mono = monospace_family();

    let mut el = div()
        .h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1p5()
        // Hairline separator between diff rows.
        .border_b_1()
        .border_color(theme.border.opacity(0.6))
        .rounded_md()
        .when(row.differ, |d| d.bg(theme.warning.opacity(0.12)));
    el = el.child(
        div()
            .text_sm()
            .font_medium()
            .font_family(mono)
            .min_w(px(112.))
            .text_ellipsis()
            .overflow_hidden()
            .child(row.name.clone()),
    );
    for (value, present) in [
        (row.global.clone(), true),
        (row.project.clone(), project_present),
        (row.effective.clone(), true),
    ] {
        let text = if !present {
            t!("env.diff_select_project").to_string()
        } else {
            value.clone().unwrap_or_else(|| "—".to_string())
        };
        el = el.child(cell(text, mono, theme.foreground, false));
    }
    el.into_any_element()
}

fn cell(text: String, mono: &'static str, color: gpui_kit::Hsla, header: bool) -> AnyElement {
    div()
        .text_sm()
        .when(header, |d| d.font_medium())
        .min_w_0()
        .flex_1()
        .text_ellipsis()
        .overflow_hidden()
        .font_family(mono)
        .text_color(color)
        .child(text)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Add / edit dialog
// ---------------------------------------------------------------------------

/// Body of the 「新增 / 编辑变量」 dialog (rule 10: the equivalent command and
/// its scope are shown BEFORE anything is written). Called from INSIDE the
/// dialog builder — it re-runs on every rebuild and reads the LIVE dialog
/// state through the weak entity, so typing and the scope choice react in
/// place. Nothing here writes anything; the write is `run_set_env` on OK.
pub fn edit_dialog_body(weak: WeakEntity<RootView>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let mono = monospace_family();

    let view = weak.upgrade().map(|v| v.read(cx));
    let Some(view) = view else {
        return div().into_any_element();
    };
    let Some(dialog) = &view.env_dialog else {
        return div().into_any_element();
    };
    let editing = dialog.editing.clone();
    let scope_global = dialog.scope_global;
    let has_project = view.state.current_project_root().is_some();

    let typed_name = dialog.name_input.read(cx).value().to_string();
    let typed_value = dialog.value_input.read(cx).value().to_string();
    // Editing keeps the name fixed (renaming would be unset + set across two
    // files; doing it in one replace is M4+ work): the resolved name is BOTH
    // what the preview shows and what the OK handler writes.
    let effective_name = resolved_env_name(editing.as_deref(), &typed_name);
    let effective_root: Option<String> = view
        .state
        .current_project_root()
        .map(|p| p.to_string_lossy().into_owned());

    let mut col = div().v_flex().gap_2().min_w_0().w(px(420.0));

    // Name: input when adding; a static line when editing.
    if let Some(editing) = &editing {
        col = col
            .child(field_label(cx, t!("env.name").to_string()))
            .child(div().text_sm().font_family(mono).child(editing.clone()));
    } else {
        col = col
            .child(field_label(cx, t!("env.name").to_string()))
            .child(div().w_full().child(Input::new(&dialog.name_input).small()));
        // Inline validation (rule: fail on our side first).
        if !valid_env_name(&effective_name) {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(t!("env.invalid_name").to_string()),
            );
        }
    }

    col = col
        .child(field_label(cx, t!("env.value").to_string()))
        .child(
            div()
                .w_full()
                .child(Input::new(&dialog.value_input).small()),
        );

    // Write scope. The project option only exists when a project is in
    // context — a project-labeled toggle with none would lie about the
    // destination file. Editing keeps the declaration's own scope.
    col =
        col.child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(field_label(cx, t!("env.edit_scope").to_string()).into_any_element())
                .when(!has_project || editing.is_some(), |d| {
                    d.child(div().text_xs().text_color(theme.muted_foreground).child(
                        if scope_global {
                            t!("env.scope_global").to_string()
                        } else {
                            t!("env.scope_project").to_string()
                        },
                    ))
                }),
        );
    if has_project && editing.is_none() {
        col = col.child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    Button::new("env-scope-global")
                        .outline()
                        .small()
                        .selected(scope_global)
                        .label(t!("env.scope_global").to_string())
                        .on_click({
                            let weak = weak.clone();
                            move |_, _, cx| set_scope(&weak, true, cx)
                        }),
                )
                .child(
                    Button::new("env-scope-project")
                        .outline()
                        .small()
                        .selected(!scope_global)
                        .label(t!("env.scope_project").to_string())
                        .on_click({
                            let weak = weak.clone();
                            move |_, _, cx| set_scope(&weak, false, cx)
                        }),
                ),
        );
    }

    // Preview of the exact equivalent command; mirrored from the write path.
    col = col
        .child(field_label(cx, t!("env.preview_cmd").to_string()))
        .child(
            div()
                .text_sm()
                .font_family(mono)
                .min_w_0()
                .text_ellipsis()
                .overflow_hidden()
                .child(set_command_preview(
                    &effective_name,
                    &typed_value,
                    scope_global,
                    effective_root.as_deref(),
                    dialog.editing_source.as_deref(),
                )),
        );

    col.into_any_element()
}

fn field_label(cx: &App, text: String) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// Flip the dialog's write-scope choice on the LIVE dialog state. When no
/// dialog is open the click is a no-op (a dialog without state can never
/// write config).
fn set_scope(weak: &WeakEntity<RootView>, global: bool, cx: &mut App) {
    if let Some(view) = weak.upgrade() {
        view.update(cx, |this, cx| {
            if let Some(dialog) = &mut this.env_dialog {
                dialog.scope_global = global;
            }
            cx.notify();
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn decl(name: &str, value: &str, source: &str, ambiguous: bool) -> EnvDeclaration {
        EnvDeclaration {
            name: name.into(),
            value: value.into(),
            source: PathBuf::from(source),
            ambiguous,
        }
    }

    fn var(name: &str, value: &str) -> EnvVar {
        EnvVar {
            name: name.into(),
            value: value.into(),
            source: None,
            tool: None,
        }
    }

    // ---- valid_env_name ----

    #[test]
    fn valid_names_accepted() {
        assert!(valid_env_name("A"));
        assert!(valid_env_name("_x1"));
        assert!(valid_env_name("NODE_ENV"));
        assert!(valid_env_name("aB_9"));
    }

    #[test]
    fn invalid_names_refused() {
        assert!(!valid_env_name(""));
        assert!(!valid_env_name("1A"));
        // Spaces are always invalid.
        assert!(!valid_env_name("A B"));
        // `-` never occurs in env names mise accepts.
        assert!(!valid_env_name("A-B"));
        assert!(!valid_env_name("A.B"));
    }

    // ---- config_path_tokens / classify_path_segment ----

    #[test]
    fn path_value_splits_into_tokens() {
        let rows = vec![decl(
            "PATH",
            "/proj/bin:$HOME/bin:/usr/bin",
            "/p/m.toml",
            false,
        )];
        let tokens = config_path_tokens(&rows);
        assert!(tokens.contains("/proj/bin"));
        assert!(tokens.contains("$HOME/bin"));
        assert!(tokens.contains("/usr/bin"));
    }

    #[test]
    fn whole_path_values_count_as_tokens() {
        let rows = vec![decl("JAVA_HOME", "/opt/jdk", "/p/m.toml", false)];
        assert!(config_path_tokens(&rows).contains("/opt/jdk"));
    }

    #[test]
    fn tool_bin_segment_classified() {
        let tokens = HashSet::new();
        let seg = "/Users/x/.local/share/mise/installs/node/20.20.2/bin";
        assert_eq!(
            classify_path_segment(seg, &tokens),
            PathSegmentKind::ToolBin("node".into())
        );
    }

    #[test]
    fn config_segment_classified() {
        let tokens = HashSet::from(["/proj/bin".to_string()]);
        assert_eq!(
            classify_path_segment("/proj/bin", &tokens),
            PathSegmentKind::Config
        );
    }

    #[test]
    fn system_segment_falls_through() {
        let tokens = HashSet::from(["/opt/jdk".to_string()]);
        assert_eq!(
            classify_path_segment("/usr/bin", &tokens),
            PathSegmentKind::System
        );
        assert_eq!(
            classify_path_segment("/opt/jdk", &tokens),
            PathSegmentKind::Config
        );
    }

    #[test]
    fn path_hints_annotate_every_segment() {
        let tokens = HashSet::from(["/proj/bin".to_string()]);
        let hints = path_segment_hints(
            "/proj/bin:/Users/x/.local/share/mise/installs/python/3.13/bin:/usr/bin",
            &tokens,
        );
        assert_eq!(
            hints,
            vec![
                SegmentHint {
                    segment: "/proj/bin".into(),
                    kind: PathSegmentKind::Config
                },
                SegmentHint {
                    segment: "/Users/x/.local/share/mise/installs/python/3.13/bin".into(),
                    kind: PathSegmentKind::ToolBin("python".into()),
                },
                SegmentHint {
                    segment: "/usr/bin".into(),
                    kind: PathSegmentKind::System
                },
            ]
        );
    }

    #[test]
    fn path_kind_label_keys() {
        assert_eq!(
            path_kind_label_key(&PathSegmentKind::ToolBin("node".into())),
            "env.path_tool"
        );
        assert_eq!(
            path_kind_label_key(&PathSegmentKind::Config),
            "env.path_config"
        );
        assert_eq!(
            path_kind_label_key(&PathSegmentKind::System),
            "env.path_system"
        );
    }

    // ---- set_command_preview ----

    #[test]
    fn preview_global_command() {
        assert_eq!(
            set_command_preview("NODE_ENV", "production", true, Some("/proj"), None),
            "mise set --global NODE_ENV=production"
        );
    }

    #[test]
    fn preview_project_command_carries_root() {
        assert_eq!(
            set_command_preview("FOO", "bar", false, Some("/my dir"), None),
            "mise -C /my dir set FOO=bar"
        );
    }

    #[test]
    fn preview_quotes_value_with_spaces() {
        assert_eq!(
            set_command_preview("B", "x y z", true, None, None),
            "mise set --global 'B=x y z'"
        );
    }

    // ---- unset_target (delete resolution) ----

    #[test]
    fn delete_targets_the_row_source_file_first() {
        // A global-sourced row on a PROJECT page must still target the exact
        // file — the page scope (`-C proj`) would silently remove nothing.
        assert_eq!(
            unset_target(Some("/home/x/.config/mise/config.toml"), false),
            UnsetTarget::File("/home/x/.config/mise/config.toml".into())
        );
    }

    #[test]
    fn delete_falls_back_to_page_scope_without_source() {
        assert_eq!(
            unset_target(None, false),
            UnsetTarget::Scope { global: false }
        );
        assert_eq!(
            unset_target(None, true),
            UnsetTarget::Scope { global: true }
        );
        assert_eq!(
            unset_target(Some("  "), true),
            UnsetTarget::Scope { global: true }
        );
    }

    // ---- project_scope_declarations (diff column) ----

    #[test]
    fn diff_project_column_excludes_global_rows() {
        // The current-scope ALSO lists global declarations; a global-sourced
        // row must not appear in the project column.
        let root = Path::new("/proj");
        let rows = vec![
            decl("ENGINE", "3.0.0", "~/.config/mise/config.toml", false),
            decl("FOO", "bar", "/proj/mise.toml", false),
        ];
        let project = project_scope_declarations(&rows, root);
        assert_eq!(project.len(), 1);
        assert_eq!(project[0].name, "FOO");
    }

    #[test]
    fn diff_global_only_variable_is_flagged() {
        // After the filter, the global-only variable has no project value at
        // all — the diff flags it (global value vs project absence).
        let rows = vec![decl("ENGINE", "3.0.0", "~/.config/mise/config.toml", false)];
        let empty: Vec<EnvDeclaration> = vec![];
        let eff: Vec<EnvVar> = vec![];
        let diffs = compute_diff(
            Some(&rows),
            Some(&project_scope_declarations(&empty, Path::new("/proj"))),
            &eff,
        );
        assert!(diffs[0].differ);
        assert_eq!(diffs[0].project, None);
    }

    // ---- group_by_source ----

    #[test]
    fn groups_by_source_sorted() {
        let rows = vec![
            decl("A", "1", "/proj/mise.toml", false),
            decl("C", "3", "~/.config/mise/config.toml", false),
            decl("B", "2", "/proj/mise.toml", false),
        ];
        let groups = group_by_source(&rows);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "/proj/mise.toml");
        assert_eq!(groups[1].0, "~/.config/mise/config.toml");
        let names: Vec<&str> = groups[0].1.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["A", "B"]);
    }

    #[test]
    fn empty_rows_group_to_nothing() {
        assert!(group_by_source(&[]).is_empty());
    }

    // ---- compute_diff ----

    #[test]
    fn diff_equal_values_do_not_differ() {
        let g = vec![decl("A", "1", "/g.toml", false)];
        let p = vec![decl("A", "1", "/p.toml", false)];
        let eff = vec![var("A", "1")];
        let rows = compute_diff(Some(&g), Some(&p), &eff);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].differ);
    }

    #[test]
    fn diff_project_only_value_differs() {
        let g: Vec<EnvDeclaration> = vec![];
        let p = vec![decl("P_ONLY", "x", "/p.toml", false)];
        let eff = vec![var("P_ONLY", "x")];
        let rows = compute_diff(Some(&g), Some(&p), &eff);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].global, None);
        assert_eq!(rows[0].project.as_deref(), Some("x"));
        assert!(rows[0].differ);
    }

    #[test]
    fn diff_value_conflict_flagged() {
        let g = vec![decl("A", "1", "/g.toml", false)];
        let p = vec![decl("A", "2", "/p.toml", false)];
        let eff = vec![var("A", "2")];
        let rows = compute_diff(Some(&g), Some(&p), &eff);
        assert!(rows[0].differ);
        assert_eq!(rows[0].global.as_deref(), Some("1"));
        assert_eq!(rows[0].project.as_deref(), Some("2"));
        assert_eq!(rows[0].effective.as_deref(), Some("2"));
    }

    #[test]
    fn diff_effective_vs_declared_mismatch_flagged() {
        let g = vec![decl("A", "1", "/g.toml", false)];
        let p = vec![decl("A", "1", "/p.toml", false)];
        let eff = vec![var("A", "surprising")];
        let rows = compute_diff(Some(&g), Some(&p), &eff);
        assert!(rows[0].differ);
    }

    #[test]
    fn diff_system_var_only_in_effective_not_flagged() {
        // A variable that exists at runtime but no config declares (TERM, …)
        // is not a DIFFERENCE between declarations and the environment.
        let eff = vec![var("TERM", "xterm-256color")];
        let rows = compute_diff(Some(&[]), Some(&[]), &eff);
        assert!(!rows[0].differ);
    }

    #[test]
    fn diff_neutral_project_column() {
        // On a global page the project column has no data at all: absence is
        // the neutral state, and the still-compared global/effective pair
        // matching means no difference.
        let g = vec![decl("A", "1", "/g.toml", false)];
        let eff = vec![var("A", "1")];
        let rows = compute_diff(Some(&g), None, &eff);
        assert!(!rows[0].differ);
        assert_eq!(rows[0].project, None);
    }

    #[test]
    fn diff_sorted_and_deduplicated() {
        let g = vec![decl("B", "1", "/g.toml", false)];
        let p = vec![decl("A", "1", "/p.toml", false)];
        let eff = vec![var("B", "1")];
        let names: Vec<String> = compute_diff(Some(&g), Some(&p), &eff)
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, vec!["A", "B"]);
    }

    // ---- edit_prefill (Fix C: prefill must use the exact value) ----

    #[test]
    fn prefill_non_ambiguous_uses_declared_value() {
        let d = decl("A", "plain", "/p/m.toml", false);
        assert_eq!(edit_prefill(None, &d), Prefill::Declared("plain".into()));
    }

    #[test]
    fn prefill_ambiguous_with_exact_uses_exact_value() {
        let d = decl("A", "a b", "/p/m.toml", true);
        assert_eq!(
            edit_prefill(Some("a  b"), &d),
            Prefill::Exact("a  b".into())
        );
    }

    #[test]
    fn prefill_ambiguous_without_exact_refuses() {
        let d = decl("A", "a b", "/p/m.toml", true);
        assert_eq!(edit_prefill(None, &d), Prefill::Refuse);
    }

    #[test]
    fn prefill_redacted_never_plans_the_placeholder() {
        // mise redacts by default; the literal `[redacted]` must not become
        // the prefill — not even when the row parsed unambiguously.
        let d = decl("SECRET", "[redacted]", "/p/m.toml", false);
        assert_eq!(edit_prefill(None, &d), Prefill::Refuse);
        // An exact read that came back redacted also refuses.
        assert_eq!(edit_prefill(Some("[redacted]"), &d), Prefill::Refuse);
        assert_eq!(
            edit_prefill(Some("hunter2"), &d),
            Prefill::Exact("hunter2".into())
        );
    }

    // ---- diff vs `[redacted]` rows ----

    #[test]
    fn diff_redacted_rows_never_create_false_conflicts() {
        let red = decl("SECRET", "[redacted]", "/p/m.toml", true);
        let eff = vec![var("SECRET", "hunter2")];
        // Redacted placeholders are not values: a column holding only the
        // placeholder counts as ABSENT, so two redacted columns never
        // conflict, and the effective value is not flagged against one.
        let g_red = vec![decl("SECRET", "[redacted]", "/g.toml", true)];
        let rows = compute_diff(Some(&g_red), Some(&[red]), &eff);
        assert!(!rows[0].differ);
        assert_eq!(rows[0].global, None);
        assert_eq!(rows[0].project, None);
    }

    // ---- reveal / copy state ----

    #[test]
    fn reveal_default_is_hidden() {
        let revealed: HashSet<String> = HashSet::new();
        assert!(!is_revealed(false, &revealed, "API_KEY"));
        assert!(!copy_allowed(false, &revealed, "API_KEY"));
    }

    #[test]
    fn per_row_reveal_and_copy() {
        let mut revealed = HashSet::new();
        revealed.insert("API_KEY".to_string());
        assert!(is_revealed(false, &revealed, "API_KEY"));
        assert!(!is_revealed(false, &revealed, "OTHER"));
        assert!(copy_allowed(false, &revealed, "API_KEY"));
    }

    #[test]
    fn show_all_reveals_and_allows_copy() {
        let revealed: HashSet<String> = HashSet::new();
        assert!(is_revealed(true, &revealed, "ANY"));
        assert!(copy_allowed(true, &revealed, "ANY"));
    }

    #[test]
    fn hidden_value_masks_display() {
        assert_eq!(display_value("secret", false), "••••••");
        assert_eq!(display_value("secret", true), "secret");
    }
}
