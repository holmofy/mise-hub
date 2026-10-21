use crate::app::RootView;
use crate::state::{Loadable, apply_status_key};
use crate::theme::monospace_family;
use gpui_kit::base::Selectable as _;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::*;
use mise_core::types::ToolInfo;
use mise_core::{Scope, ToolRow};
use rust_i18n::t;

/// Render the detail pane for a single installed tool row: the basic
/// `mise ls` fields always render; `mise tool <name> --json` data (loaded
/// asynchronously and guarded by `info_tool`) enriches the pane, and a
/// failure keeps the raw error visible without hiding the basic fields
/// (AGENTS.md rule 9). Actions: 「应用版本」 (preview + confirm dialog, rule
/// 4), 「卸载」 (destructive, confirmed, rule 10), and 「升级」 when the tool
/// is in the outdated list. Buttons disable with a running label while a
/// run for the same spec is in flight — never a silently dead button.
#[allow(clippy::too_many_arguments)]
pub fn render(
    cx: &mut Context<RootView>,
    row: &ToolRow,
    info: &Loadable<ToolInfo>,
    info_tool: Option<&str>,
    can_upgrade: bool,
    busy_use: bool,
    busy_uninstall: bool,
    busy_upgrade: bool,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();
    let apply_spec = format!("{}@{}", row.name, row.version);
    // Owned copies for the event closures: a published element's closures
    // must not borrow the (short-lived) `row` reference.
    let uninstall_name = row.name.clone();
    let uninstall_version = row.version.clone();
    let upgrade_name = row.name.clone();
    // Apply targets the scope the tool was listed in: project page → project
    // config, anything else → global config (rule 2; `use_scope` resolves
    // the exact scope again when the flow runs).
    let apply_global = row.scope == Scope::Global || apply_global_root(cx);

    let mut pane = div()
        .flex_1()
        .h_full()
        .v_flex()
        .p_4()
        .gap_3()
        .bg(theme.background)
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(div().text_lg().font_semibold().child(row.name.clone()))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(format!("{} {}", t!("tools.version"), row.version)),
                ),
        )
        .child(field(
            cx,
            t!("tools.scope").to_string(),
            scope_label(row.scope),
        ))
        .child(field(
            cx,
            t!("tools.source").to_string(),
            row.source_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "—".to_string()),
        ))
        .child(field(
            cx,
            t!("tools.install_path").to_string(),
            row.install_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "—".to_string()),
        ));

    // Rule 4 copy: 已生效 only when the refreshed row is active; otherwise
    // the honest 「已安装 · 未生效」 stays until an apply succeeds.
    pane = pane.child(field(
        cx,
        t!("tools.status").to_string(),
        t!(apply_status_key(row.active)).to_string(),
    ));

    // `mise tool` enrichment. Only shows when the loaded info belongs to
    // this tool (stale results from a previous selection are dropped).
    if info_tool == Some(row.name.as_str()) {
        pane = pane
            .child(div().border_t_1().border_color(theme.border))
            .child(details_fields(cx, info));
    }

    pane = pane.child(div().flex_1()).child(
        div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("apply")
                            .primary()
                            .small()
                            .disabled(busy_use)
                            .label(t!("tools.apply").to_string())
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.prompt_apply(apply_spec.clone(), apply_global, window, cx);
                            })),
                    )
                    .when(can_upgrade, |d: Div| {
                        let name = upgrade_name.clone();
                        d.child(
                            Button::new("upgrade")
                                .outline()
                                .small()
                                .disabled(busy_upgrade)
                                .label(t!("upgrade.row").to_string())
                                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                    this.run_tool_upgrade(&name, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("uninstall")
                            .danger()
                            .outline()
                            .small()
                            .disabled(busy_uninstall)
                            .label(t!("tools.uninstall").to_string())
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.prompt_uninstall(
                                    uninstall_name.clone(),
                                    uninstall_version.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .when(busy_use || busy_uninstall || busy_upgrade, |d| {
                d.child(
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(Spinner::new().xsmall())
                        .child(t!("tools.apply_running").to_string()),
                )
            }),
    );

    pane
}

/// The `mise tool <name> --json` section of the detail pane: loading state,
/// the enriched fields when ready, or the raw error on failure (rule 9).
fn details_fields(cx: &Context<RootView>, info: &Loadable<ToolInfo>) -> impl IntoElement + use<> {
    let theme = cx.theme();
    match info {
        Loadable::Loading | Loadable::Idle => div()
            .h_flex()
            .gap_2()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(Spinner::new().xsmall())
            .child(t!("tools.info_loading").to_string())
            .into_any_element(),
        Loadable::Failed(err) => div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(t!("tools.info_failed").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(err.clone()),
            )
            .into_any_element(),
        Loadable::Ready(info) => div()
            .v_flex()
            .gap_3()
            .when_some(info.description.clone(), |d, description| {
                d.child(field(cx, t!("tools.description").to_string(), description))
            })
            .when_some(info.url.clone(), |d, url| {
                d.child(field(cx, t!("tools.url").to_string(), url))
            })
            .child(field(
                cx,
                t!("tools.backend").to_string(),
                info.backend.clone(),
            ))
            .child(field(
                cx,
                t!("tools.installed_versions").to_string(),
                info.installed_versions.join(", "),
            ))
            .when_some(
                info.config_source.as_ref().and_then(|s| s.path.clone()),
                |d, path| {
                    d.child(field(
                        cx,
                        t!("tools.config_source").to_string(),
                        path.display().to_string(),
                    ))
                },
            )
            .into_any_element(),
    }
}

/// The body of the 「应用版本」 confirm dialog (rule 10: impact shown before
/// acting). Called from INSIDE the dialog's builder — it re-runs on every
/// redraw and reads the LIVE flow through the weak entity, so the async
/// preview streams into the open dialog and the pin choice is reactive.
/// Nothing here writes anything; the write is `run_use` on OK (rule 3: mise
/// decides which file changes, we only show what mise reported).
pub fn apply_dialog_body(
    weak: gpui_kit::WeakEntity<RootView>,
    cx: &App,
) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let mono = monospace_family();

    let flow = weak
        .upgrade()
        .map(|view| view.read(cx).apply_flow.clone())
        .unwrap_or(None);
    let (global, pin, preview) = match &flow {
        Some(flow) => (flow.global, flow.pin, Some(flow.preview.clone())),
        None => (true, false, None),
    };

    let mut col = div().v_flex().gap_2().min_w_0();
    // Scope statement: which config the write goes to (never guessed after
    // the fact — mise's `--dry-run` output decides what config files change,
    // this line is only which scope the command targets).
    col = col.child(div().text_sm().child(if global {
        t!("tools.apply_note_global").to_string()
    } else {
        t!("tools.apply_note_project").to_string()
    }));

    // Pin choice: two mutually exclusive toggle buttons; state kept in the
    // live flow so the eventual `mise use [--pin]` matches what the user
    // SEES selected, not what was defaulted when the dialog opened.
    col = col.child(
        div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("tools.apply_pin_choice").to_string()),
            )
            .child(
                Button::new("pin")
                    .outline()
                    .small()
                    .selected(pin)
                    .label(t!("tools.apply_pin").to_string())
                    .on_click({
                        let weak = weak.clone();
                        move |_, _, cx| toggle_pin(&weak, true, cx)
                    }),
            )
            .child(
                Button::new("fuzzy")
                    .outline()
                    .small()
                    .selected(!pin)
                    .label(t!("tools.apply_range").to_string())
                    .on_click(move |_, _, cx| toggle_pin(&weak, false, cx)),
            ),
    );

    // What will be installed + which config files change, from the dry run;
    // then the raw output verbatim.
    col = col.child(match preview {
        None | Some(Loadable::Loading) | Some(Loadable::Idle) => div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Spinner::new().xsmall())
                    .child(t!("tools.apply_preview_loading").to_string()),
            )
            // Rule 10 honesty: while the dry run has not produced a plan,
            // OK cannot write anything — say so explicitly.
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("tools.apply_preview_pending_note").to_string()),
            )
            .into_any_element(),
        Some(Loadable::Failed(err)) => div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(t!("tools.apply_preview_failed").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("tools.apply_preview_pending_note").to_string()),
            )
            .child(div().text_xs().child(err))
            .into_any_element(),
        Some(Loadable::Ready(preview)) => {
            let mut outcomes = div().v_flex().gap_1().min_w_0();
            if preview.install.is_empty() {
                outcomes = outcomes.child(
                    div()
                        .text_xs()
                        .child(t!("tools.apply_will_install_none").to_string()),
                );
            } else {
                outcomes = outcomes.child(
                    div()
                        .text_xs()
                        .child(t!("tools.apply_will_install").to_string()),
                );
                for install_spec in &preview.install {
                    outcomes = outcomes.child(
                        div()
                            .text_xs()
                            .font_family(mono)
                            .child(format!("· {install_spec}")),
                    );
                }
            }
            if preview.config_updates.is_empty() {
                outcomes = outcomes.child(
                    div()
                        .text_xs()
                        .child(t!("tools.apply_config_updates_none").to_string()),
                );
            } else {
                outcomes = outcomes.child(
                    div()
                        .text_xs()
                        .child(t!("tools.apply_config_updates").to_string()),
                );
                for path in &preview.config_updates {
                    outcomes = outcomes.child(
                        div()
                            .text_xs()
                            .font_family(mono)
                            .child(format!("· {}", path.display())),
                    );
                }
            }
            outcomes = outcomes.child(
                div()
                    .mt_1()
                    .v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("tools.apply_raw").to_string()),
                    )
                    .child(div().text_xs().font_family(mono).child(preview.raw.clone())),
            );
            outcomes.into_any_element()
        }
    });

    col.into_any_element()
}

/// Flip the dialog's pin choice on the LIVE flow. When no flow is open the
/// click is a no-op (a dialog without state can never write config).
fn toggle_pin(weak: &gpui_kit::WeakEntity<RootView>, pin: bool, cx: &mut App) {
    if let Some(view) = weak.upgrade() {
        view.update(cx, |this, cx| {
            if let Some(flow) = &mut this.apply_flow {
                flow.pin = pin;
            }
            cx.notify();
        });
    }
}
fn field(cx: &App, label: String, value: String) -> impl IntoElement + use<> {
    let theme = cx.theme();
    div()
        .v_flex()
        .gap_0p5()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(div().text_sm().child(value))
}

fn scope_label(scope: Scope) -> String {
    match scope {
        Scope::Project => t!("tools.project").to_string(),
        Scope::Global => t!("tools.global").to_string(),
    }
}

/// Whether the apply flow has a project in context: a project page applies
/// to the project config, everything else defaults to the global config
/// (the same resolution `use_scope` re-runs when the run actually starts).
fn apply_global_root(cx: &Context<RootView>) -> bool {
    cx.entity().read(cx).state.current_project_root().is_none()
}
