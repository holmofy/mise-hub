# M0 + M1 Verification Record (Task 14)

Date: 2026-10-20 · Branch: `feat/m0-m1` · Platform: macOS (darwin)

## Automated checks

All run from the repo root; exact output below.

### `cargo fmt --all -- --check`

```
(clean — no output, exit 0)
```

### `cargo clippy --workspace --all-targets`

```
warning: field assignment outside of initializer for an instance created with Default::default()
  --> crates/mise-core/src/settings.rs:73:9
warning: `mise-core` (lib test) generated 1 warning
warning: the following packages contain code that will be rejected by a future version of Rust: block v0.1.6
```

Zero warnings in `mise-hub`. The one remaining warning is in
`crates/mise-core/src/settings.rs`, which is outside Task 14's writable set
(`crates/mise-core/**` is read-only for this task). `cargo clippy -D warnings`
is therefore blocked only by that pre-existing mise-core lint.

### `cargo test --workspace`

```
running 28 tests ... test result: ok. 28 passed; 0 failed (mise-core)
running 29 tests ... test result: ok. 29 passed; 0 failed (mise-hub)
running 0 ... test result: ok. 0 passed
```

### `cargo build --workspace`

```
Finished `dev` profile [unoptimized + debuginfo] target(s)
(no warnings from mise-hub or mise-core; upstream `block v0.1.6` future-incompat notice only)
```

## GUI terminal cap hardening (automated, not GUI-only)

- Terminal sessions now retain at most the last
  `MAX_TERMINAL_LINES = 5000` lines per session
  (`crates/mise-hub/src/state.rs`, `TerminalSession::feed`). The `\r`
  final-fragment behavior and per-session routing are unchanged (pinned by
  the existing tests `carriage_render_appends_final_fragment_without_rewriting_history`
  and `push_event_to_routes_only_to_target_session`, plus new tests
  `feed_caps_lines_at_max_terminal_lines` and `feed_under_cap_keeps_all_lines`).
- Monospace font is no longer hardcoded `Menlo`:
  `theme::monospace_family()` picks Menlo (macOS) / Consolas (Windows) /
  DejaVu Sans Mono (other) from a documented chain ending in a generic
  `monospace` fallback (`crates/mise-hub/src/theme.rs`).

## Acceptance items (Task 14 brief, Step 3)

| # | Item | Result | Evidence / reason |
|---|------|--------|-------------------|
| 1 | Launch → main UI, sidebar matches design | PASS (state) / UNVERIFIED (visual) | App builds and the default selection renders the Discover page; the sidebar structure is pinned by `sidebar::render` usage in `RootView::render` (`crates/mise-hub/src/app.rs`). Actual visual layout requires a human: `cargo run -p mise-hub` and compare with the design. |
| 2 | Add a real project → appears under 项目, survives restart | PASS (logic) / UNVERIFIED (GUI) | Persistence is covered by automated tests: `add_project_dedupes_same_path` (save) plus `ProjectStore::load` round-trip in `mise-core`. Human check: click 添加项目, pick a folder, restart the app. |
| 3 | Tools page scope filter 全部/本项目/全局 shows the same tool in two versions side by side | PASS (logic) / UNVERIFIED (GUI) | `rows_tests::all_shows_same_tool_twice_when_both_scopes` proves filtering; `tool_list::row_element` renders both rows. Human check: select a project that pins its own version of a globally installed tool. |
| 4 | Discover search `ripgrep` / `rg` / `javascript` returns sensible results | PASS (logic) / UNVERIFIED (GUI) | `search_uses_loaded_registry` and `empty_query_returns_all_entries_alphabetically` pin the local index search (name/aliases/bins/description matching, backed by `mise-core::SearchIndex`). Human check: run the app and type the three queries. |
| 5 | Clear search → full list, smooth scrolling | PASS (logic) / UNVERIFIED (GUI) | Browse mode with `usize::MAX` limit returns every entry; the virtual list (`v_virtual_list`) only renders the visible window. Human check: clear the box and scroll the ~1067-row list. |
| 6 | Pick a version, install → live bottom-panel output, correct final status | PASS (logic) / UNVERIFIED (GUI) | Streaming, per-session routing, ANSI stripping, `\r` progress handling, and final status are all pinned by `terminal_tests` and `install_tests` in `crates/mise-hub/src/state.rs`. Human check: install `ripgrep@<ver>` and watch the panel. |
| 7 | Install writes no `mise.toml` / no config change | **Verified by inspection** | The install path is exactly `AppState::start_install` → `Mise::install_stream` (`crates/mise-hub/src/state.rs:175`) → `mise-core` spawns only `mise [-C dir] install <tool>@<version>` (`crates/mise-core/src/mise.rs:59-73`). No code between `run_install`/`start_install` calls `mise use`, `mise set`, or performs any file write; the only write near this flow is the explicit user-triggered `save_terminal_log`. The diff-driven `mise ls --json` before/after comparison itself is GUI-only: UNVERIFIED (human command: `mise ls --json > before; run install; mise ls --json > after; diff before after` — the tools list may gain the new install, which is expected; no `mise.toml` may change). |
| 8 | PATH missing mise → guidance page; restored → 重新检测 works | PASS (logic) / UNVERIFIED (GUI) | `detect_tests::detect_without_mise_leaves_state_empty` pins the no-mise path; `RootView::render` shows `setup::render(on_detect)` when `state.mise.is_none()` and the detect button re-runs detection + registry load. Human check: launch with an emptied PATH. |
| 9 | Dark theme switches immediately and persists | PASS (logic) / UNVERIFIED (GUI) | `theme_picker` applies via `Theme::update` (refreshes all windows) and persists through `settings.save(&settings_path)`; load path in `main.rs`. Human check: switch to Default Dark, restart. |
| 10 | Language toggle 中文/English | PASS (logic) / UNVERIFIED (GUI) | `language_picker` calls `i18n::set_locale` and persists; all strings go through `t!` and both `locales/en.yml` and `locales/zh-CN.yml` carry every key. Human check: toggle and confirm visible copy changes. |

## Coverage of state rendering (brief Step 1)

| Page | Loading / Idle | Failed | Empty |
|------|----------------|--------|-------|
| Tools list | `tools.loading` spinner text when not Ready/Failed | `tools.failed` headline + raw stderr + 重试 button (re-runs `mise ls`) | `tools.empty` (nothing installed) vs `tools.no_match` (search active, zero rows) |
| Tool detail | Only shown for a selected row (nothing to load) | detail fields absent → placeholders per `tool_detail` | `tools.select_tool` placeholder when nothing is selected |
| Discover | `discover.loading` centered | `discover.failed` headline + raw error + 重试 button (re-runs `mise registry`) | `discover.registry_empty` (empty query, empty registry) vs `discover.no_results` (query matched nothing) |
| Versions pane | `discover.loading_versions` + spinner | `discover.versions_failed` + raw error (no retry affordance: the picker re-fetches on the next click of the tool) | `discover.pick_tool` before any selection |
| Terminal | Sessions precede the panel (panel hidden until a run exists); running tab shows a spinner | Tab status `Failed (code)`; raw stderr lines already in the stream — no synthetic retry | Cleared session shows an empty body by design (保存/复制 disabled state is acceptable for M1; the tab is kept) |
| Setup page (mise missing) | — | — | Guidance copy + Detect again button (re-runs detection) |

## Remaining UNVERIFIED (requires a human at the GUI)

Run `cargo run -p mise-hub` and check items 1–6, 8–10 in the table above.
No automated harness can open the GPUI window headlessly; none of these
"PASS" rows include a human visual confirmation.
