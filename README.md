# mise-hub

A desktop GUI for [mise](https://mise.jdx.dev) — for people who already use
mise (or want to) but never remember the commands.

**mise-hub does not reimplement mise.** Every operation goes through the
`mise` CLI as a subprocess, so the app always agrees with what the user
would get by typing the same command in a terminal. What the app adds is
*discoverability*: the tools you have, the versions you can install, the
environment variables mise actually exports, the tasks a project defines —
all visible, all clickable, all in Chinese and English.

> 中文简介：mise-hub 是 mise 的图形界面。它不重写 mise，所有操作都通过
> `mise` 命令行子进程完成；它补的是「可发现性」——已安装的工具、可安装的
> 版本、生效中的环境变量、项目任务，都能在界面里看见并一键操作，界面
> 中英双语。

---

## Features

| Area | What you get |
|---|---|
| Discovery | Search the local tool registry (English keywords, instant, offline), browse every tool, see its versions, install it, watch the progress in the bottom panel. An explicit "Deep search" entry runs `mise search --all` online. |
| Tool management | Upgrade (with `--bump`), uninstall, apply a version to the project or globally — with a dry-run preview of exactly which config file changes before anything is written. |
| Install ≠ activate | A freshly installed tool is shown as **installed · not active yet**, with explicit "Apply to this project / Apply globally" buttons. The app never pretends installing is enough. |
| Environment | Declaration view and effective view side by side, per-variable source tracing, PATH segmentation, add / edit / delete through `mise set` / `mise unset`, and a global-vs-project diff. |
| Tasks | Discover tasks (including file tasks), see their dependency tree, run them, watch them, and read the log. |
| Settings & diagnostics | Theme and language pickers, the resolved mise path and version, and one capability probe per JSON subcommand the app depends on — every failure shows mise's raw error. |
| Update check | Point the app at a release manifest; it tells you when a newer version exists and opens the download page. It never downloads or replaces the app itself. |

---

## Requirements

- [mise](https://mise.jdx.dev) on `PATH` (or in `~/.local/bin`, `/opt/homebrew/bin`,
  `/usr/local/bin`) — mise-hub locates it and shows a setup screen when it
  is missing.
- Rust 1.85+ (edition 2024) to build from source.

## Build and run

```bash
cargo build --workspace      # debug build
cargo run -p mise-hub        # run the app
cargo test --workspace       # unit tests
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
```

## Package a macOS app

```bash
scripts/package-macos.sh            # → dist/mise-hub.app
scripts/package-macos.sh --dmg      # → dist/mise-hub-<version>.dmg
```

The bundle is **unsigned**: macOS Gatekeeper blocks the first launch, so open
it via right-click → Open (or run `xattr -dr com.apple.quarantine
dist/mise-hub.app`). Add signing and notarization to the script once a
Developer ID certificate is available.

## Releases and the update manifest

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds
macOS (arm64), Windows (x64) and Linux (x64) artifacts and publishes them to
a GitHub Release.

The settings page's **Update check** reads a small JSON manifest from a URL
you configure (Settings → Update check → Manifest URL):

```json
{
  "version": "0.2.0",
  "page_url": "https://github.com/<org>/mise-hub/releases/latest"
}
```

`version` is required; `page_url` is optional (without it the app shows the
new version but cannot open a download page). The comparison is numeric
per segment, and a pre-release (`0.2.0-rc.1`) never outranks its final
release. With no URL configured the check is disabled and the button says
so.

## Platform support

| Platform | Status |
|---|---|
| macOS | Primary target; `.app` packaging provided. |
| Windows | Builds and runs; no installer is produced yet. |
| Linux | Builds and runs; the CI artifact is best-effort. |

No platform-specific paths are hardcoded in the Rust code — the mise
locator, the font chain and the browser opener all branch per platform.

## Project layout

```
crates/mise-core   # pure Rust: locating mise, calling the CLI, parsing JSON
crates/mise-hub    # GPUI desktop app: state and rendering only
locales/           # en.yml / zh-CN.yml (key sets are checked by a test)
scripts/           # packaging helpers
docs/superpowers/  # design spec, milestone plans, verification records
```

`mise-core` never imports a GPUI type; `mise-hub` never spawns `mise`
itself — every mise call goes through `mise-core`.

## Documentation

- Design spec: `docs/superpowers/specs/2026-10-19-mise-hub-design.md`
- Milestone plans and verification records: `docs/superpowers/plans/`
- Agent rules (hard constraints, red lines): `AGENTS.md`

## License

Not yet chosen — pick one before the first public release.
