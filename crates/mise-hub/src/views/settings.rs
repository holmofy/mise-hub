//! Settings & diagnostics page (M5-T2).
//!
//! Four sections, top to bottom: appearance (theme + language, persisted
//! like the header pickers), mise diagnostics (resolved path, version and
//! one capability probe per JSON subcommand the app depends on — each
//! failure shows the RAW error, AGENTS.md rule 9), app info (versions and
//! the on-disk paths this app reads/writes), and the update check (manifest
//! URL + a check that ends in 「打开下载页」 — mise-hub never downloads or
//! replaces itself).
//!
//! Everything slow (probes, the HTTP check) runs on background threads via
//! `RootView::rerun_diagnostics` / `RootView::check_for_updates`; this
//! module only renders state. All page data arrives as a [`SettingsSnapshot`]
//! — view builders must never `cx.entity().read(cx)` mid-render (the entity
//! is leased out then; a second read panics).

use crate::app::{LOCALES, RootView};
use crate::state::Loadable;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;
use mise_core::{DiagnosticsReport, UpdateCheck};
use rust_i18n::t;
use std::path::PathBuf;

/// Plain-data snapshot of everything the settings page renders, taken by
/// `RootView::settings_pane` before the builder runs (no mid-render reads).
pub struct SettingsSnapshot {
    pub theme_names: Vec<String>,
    pub current_theme: String,
    pub current_locale: String,
    pub diagnostics: Loadable<DiagnosticsReport>,
    pub diagnostics_in_flight: bool,
    pub update: Loadable<UpdateCheck>,
    pub update_in_flight: bool,
    /// The (possibly unsaved) manifest URL draft from the input entity.
    pub url_draft: String,
    pub settings_path: PathBuf,
    pub store_path: PathBuf,
}

/// Render the settings page. `update_url_input` is the shared input entity
/// holding the manifest URL draft; the check button operates on the draft,
/// 「保存」 persists it into `AppSettings`.
pub fn render(
    cx: &mut Context<RootView>,
    snapshot: SettingsSnapshot,
    update_url_input: Entity<InputState>,
) -> impl IntoElement + use<> {
    let theme = cx.theme().clone();

    let mut col = div()
        .v_flex()
        .flex_1()
        .h_full()
        .min_w_0()
        .p_3()
        .gap_2()
        .bg(theme.background);

    col = col.child(section_title(t!("settings.title").to_string()));
    col = col.child(appearance_section(cx, &snapshot));
    col = col.child(diagnostics_section(cx, &snapshot));
    col = col.child(app_info_section(&theme, &snapshot));
    col = col.child(update_section(cx, &snapshot, update_url_input));

    col
}

fn section_title(text: String) -> impl IntoElement {
    div().text_sm().font_semibold().child(text)
}

fn subsection(theme: &Theme, title: String, body: impl IntoElement) -> gpui::AnyElement {
    div()
        .v_flex()
        .gap_1p5()
        .pb_2()
        .border_b_1()
        .border_color(theme.border)
        .child(div().text_xs().opacity(0.6).child(title))
        .child(body)
        .into_any_element()
}

fn label_value(theme: &Theme, label: String, value: String) -> gpui::AnyElement {
    div()
        .h_flex()
        .items_baseline()
        .gap_2()
        .min_w_0()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .min_w_0()
                .text_ellipsis()
                .overflow_hidden()
                .child(value),
        )
        .into_any_element()
}

// -------------------------------------------------------------------
// Appearance
// -------------------------------------------------------------------

fn appearance_section(cx: &mut Context<RootView>, snapshot: &SettingsSnapshot) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let theme_names = snapshot.theme_names.clone();
    let current_theme = snapshot.current_theme.clone();
    let current_locale = snapshot.current_locale.clone();
    let weak = cx.weak_entity();

    // Theme picker: same registry list and persistence as the header
    // picker, both going through the shared `apply_theme_setting` so the
    // two entry points can never drift apart.
    let theme_picker = {
        let weak = weak.clone();
        let current = current_theme.clone();
        Button::new("settings-theme-picker")
            .outline()
            .small()
            .label(current.clone())
            .dropdown_menu(move |menu, _, _| {
                theme_names.iter().cloned().fold(menu, |menu, name| {
                    menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(name == current)
                            .on_click({
                                let weak = weak.clone();
                                let name = name.clone();
                                move |_, window, cx| {
                                    if let Some(view) = weak.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.apply_theme_setting(name.clone(), window, cx);
                                        });
                                    }
                                }
                            }),
                    )
                })
            })
    };

    // Language picker: persists and re-bakes placeholders via the shared
    // `apply_locale_setting`.
    let language_picker = {
        let weak = weak;
        let current = current_locale.clone();
        Button::new("settings-language-picker")
            .outline()
            .small()
            .label(RootView::locale_label(&current))
            .dropdown_menu(move |menu, _, _| {
                LOCALES.into_iter().fold(menu, |menu, (code, label)| {
                    menu.item(
                        PopupMenuItem::new(label)
                            .checked(code == current.as_str())
                            .on_click({
                                let weak = weak.clone();
                                move |_, window, cx| {
                                    if let Some(view) = weak.upgrade() {
                                        view.update(cx, |this, cx| {
                                            this.apply_locale_setting(code, window, cx);
                                        });
                                    }
                                }
                            }),
                    )
                })
            })
    };

    subsection(
        &theme,
        t!("settings.appearance").to_string(),
        div()
            .v_flex()
            .gap_1p5()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().child(t!("settings.theme").to_string()))
                    .child(theme_picker),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().child(t!("settings.language").to_string()))
                    .child(language_picker),
            ),
    )
    .into_any_element()
}

// -------------------------------------------------------------------
// mise diagnostics
// -------------------------------------------------------------------

fn diagnostics_section(
    cx: &mut Context<RootView>,
    snapshot: &SettingsSnapshot,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let report = snapshot.diagnostics.clone();
    let in_flight = snapshot.diagnostics_in_flight;

    let header = div()
        .h_flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_sm()
                .child(t!("settings.diagnostics").to_string()),
        )
        .child(
            Button::new("settings-rerun-probes")
                .outline()
                .small()
                .disabled(in_flight)
                .label(t!("settings.diagnostics_rerun").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.rerun_diagnostics(cx);
                })),
        );

    let body: gpui::AnyElement = match report {
        Loadable::Idle | Loadable::Loading => div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(t!("settings.diagnostics_running").to_string())
            .into_any_element(),
        Loadable::Failed(err) => div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(t!("settings.diagnostics_failed").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(err),
            )
            .into_any_element(),
        Loadable::Ready(report) => report_body(&theme, &report).into_any_element(),
    };

    subsection(
        &theme,
        t!("settings.mise").to_string(),
        div().v_flex().gap_1p5().child(header).child(body),
    )
    .into_any_element()
}

fn report_body(theme: &Theme, report: &DiagnosticsReport) -> gpui::AnyElement {
    let mut col = div()
        .v_flex()
        .gap_1p5()
        .child(label_value(
            theme,
            t!("settings.mise_path").to_string(),
            report.program.display().to_string(),
        ))
        .child(label_value(
            theme,
            t!("settings.mise_version").to_string(),
            report
                .version
                .clone()
                .unwrap_or_else(|| t!("settings.mise_version_unknown").to_string()),
        ));

    for probe in &report.probes {
        col = col.child(probe_row(theme, probe));
    }
    col.into_any_element()
}

fn probe_row(theme: &Theme, probe: &mise_core::Probe) -> gpui::AnyElement {
    let mut row = div()
        .v_flex()
        .gap_0p5()
        .px_2()
        .py_1()
        .rounded_md()
        .bg(theme.muted.opacity(0.3))
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(div().text_sm().child(probe_label(probe.name)))
                        .when(probe.network, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .px_1p5()
                                    .rounded_full()
                                    .bg(theme.info.opacity(0.15))
                                    .text_color(theme.info)
                                    .child(t!("settings.probe_network").to_string()),
                            )
                        }),
                )
                .child(if probe.ok {
                    div()
                        .text_xs()
                        .text_color(theme.success)
                        .child(t!("settings.probe_ok").to_string())
                } else {
                    div()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(t!("settings.probe_fail").to_string())
                }),
        );
    // The raw failure text, verbatim — the whole point of the probe list
    // is showing what exactly is missing (rule 9).
    if let Some(detail) = &probe.detail {
        row = row.child(
            div()
                .text_xs()
                .text_color(theme.danger.opacity(0.9))
                .overflow_hidden()
                .text_ellipsis()
                .child(detail.clone()),
        );
    }
    row.into_any_element()
}

/// i18n label for one probe name; an unknown name (never produced by the
/// current `PROBES`) renders verbatim rather than as an empty string.
fn probe_label(name: &str) -> String {
    match name {
        "registry" => t!("settings.probe.registry").to_string(),
        "ls" => t!("settings.probe.ls").to_string(),
        "config" => t!("settings.probe.config").to_string(),
        "env" => t!("settings.probe.env").to_string(),
        "tasks" => t!("settings.probe.tasks").to_string(),
        "outdated" => t!("settings.probe.outdated").to_string(),
        "ls-remote" => t!("settings.probe.ls_remote").to_string(),
        _ => name.to_string(),
    }
}

// -------------------------------------------------------------------
// App info
// -------------------------------------------------------------------

fn app_info_section(theme: &Theme, snapshot: &SettingsSnapshot) -> gpui::AnyElement {
    subsection(
        theme,
        t!("settings.app").to_string(),
        div()
            .v_flex()
            .gap_1p5()
            .child(label_value(
                theme,
                t!("settings.app_version").to_string(),
                mise_core::version().to_string(),
            ))
            .child(label_value(
                theme,
                t!("settings.settings_file").to_string(),
                snapshot.settings_path.display().to_string(),
            ))
            .child(label_value(
                theme,
                t!("settings.store_file").to_string(),
                snapshot.store_path.display().to_string(),
            )),
    )
    .into_any_element()
}

// -------------------------------------------------------------------
// Update check
// -------------------------------------------------------------------

fn update_section(
    cx: &mut Context<RootView>,
    snapshot: &SettingsSnapshot,
    update_url_input: Entity<InputState>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let update = snapshot.update.clone();
    let checking = snapshot.update_in_flight;

    let draft = snapshot.url_draft.clone();
    let url_configured = crate::state::AppState::update_check_enabled(&draft);
    let enabled = url_configured && !checking;

    let url_row = div()
        .h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w_40()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("settings.update_url").to_string()),
        )
        .child(div().flex_1().child(Input::new(&update_url_input).small()))
        .child(
            Button::new("settings-save-url")
                .outline()
                .small()
                .label(t!("common.save").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.save_update_url(cx);
                })),
        );

    let mut check_button = Button::new("settings-check-update")
        .primary()
        .small()
        .disabled(!enabled)
        .label(t!("settings.update_check").to_string())
        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
            let url = this.update_url_input.read(cx).value().trim().to_string();
            this.check_for_updates(url, cx);
        }));
    if !url_configured {
        check_button = check_button.tooltip(t!("settings.update_check_disabled").to_string());
    }

    let mut action_row = div().h_flex().items_center().gap_2().child(check_button);
    if !url_configured {
        action_row = action_row.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("settings.update_check_disabled").to_string()),
        );
    }

    let result: gpui::AnyElement = match update {
        Loadable::Idle => div().into_any_element(),
        Loadable::Loading => div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(t!("settings.update_checking").to_string())
            .into_any_element(),
        Loadable::Failed(err) => div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(t!("settings.update_failed").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(err),
            )
            .into_any_element(),
        Loadable::Ready(UpdateCheck::UpToDate) => div()
            .text_sm()
            .text_color(theme.success)
            .child(t!("settings.update_uptodate").to_string())
            .into_any_element(),
        Loadable::Ready(UpdateCheck::UpdateAvailable { latest, page_url }) => {
            let mut row = div().h_flex().items_center().gap_2().child(
                div()
                    .text_sm()
                    .text_color(theme.info)
                    .child(t!("settings.update_available", version = latest.clone()).to_string()),
            );
            // No page URL in the manifest → no button that would do
            // nothing; the version number alone still tells the user what
            // to look for.
            if let Some(url) = page_url {
                row = row.child(
                    Button::new("settings-open-update-page")
                        .primary()
                        .small()
                        .label(t!("settings.update_open_page").to_string())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.open_update_page(url.clone(), cx);
                        })),
                );
            }
            row.into_any_element()
        }
    };

    subsection(
        &theme,
        t!("settings.update").to_string(),
        div()
            .v_flex()
            .gap_1p5()
            .child(url_row)
            .child(action_row)
            .child(result),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::probe_label;

    /// Every probe name the core can emit must resolve to a translated
    /// label — an untranslated probe would render as a blank row.
    #[test]
    fn every_probe_name_has_a_label() {
        for name in [
            "registry",
            "ls",
            "config",
            "env",
            "tasks",
            "outdated",
            "ls-remote",
        ] {
            assert!(!probe_label(name).is_empty(), "no label for probe {name}");
        }
    }

    /// An unknown probe name (e.g. a future core adding a probe before the
    /// UI learns about it) renders verbatim, never an empty row.
    #[test]
    fn unknown_probe_name_renders_verbatim() {
        assert_eq!(probe_label("future-probe"), "future-probe");
    }
}
