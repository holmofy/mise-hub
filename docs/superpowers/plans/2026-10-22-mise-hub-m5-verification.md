# M5 验收记录 — mise-hub M5（收尾）

Date: 2026-10-22 · Branch `feat/m5` (HEAD `952bde3`) · Baseline mise 2026.10.x

M5 范围（用户已确认）：配置/诊断页、打包分发（本地脚本 + GitHub Actions CI）、
更新检查（版本比对 + 打开下载页，不自动下载安装）、文档、i18n 补全。
计划：`docs/superpowers/plans/2026-10-22-mise-hub-m5.md`。

## 自动化检查（真实输出）

All commands run at repo root, 2026-10-22.

### `cargo fmt --all -- --check`

```
clean (no output, exit 0)
```

### `cargo clippy --workspace --all-targets -- -D warnings`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.22s
warning: the following packages contain code that will be rejected by a future version of Rust: block v0.1.6
note: to see the problems, you can run `cargo report future-incompatibilities --id 1`
```

(The `block v0.1.6` future-incompat notice comes from a transitive
dependency, not from a clippy lint; exit code 0.)

### `cargo test --workspace`

```
     Running unittests src/lib.rs (target/debug/deps/mise_core-f9ea641b94a3ff3a)
test result: ok. 140 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src/main.rs (target/debug/deps/mise_hub-709161b73...)
test result: ok. 139 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

279 tests total, 0 failures (M4 记录基线 243；M5 新增 +36：
core 诊断 5 + 更新检查 7 + settings 兼容 1 + UI 设置页 2 + opener 5 +
状态守卫 1 + locale 键集 2 + 既有 i18n 2 + 终端标题相关断言更新)。

### `cargo build --workspace`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.40s
（exit 0）
```

### 打包脚本实跑（本机）

```
$ bash scripts/package-macos.sh --no-build
bundled: dist/mise-hub.app (version 0.1.0)
done.

$ plutil -lint dist/mise-hub.app/Contents/Info.plist
dist/mise-hub.app/Contents/Info.plist: OK

$ dist/mise-hub.app/Contents/MacOS/mise-hub &   # 启动冒烟
RUNNING (pid 85693)      # 6 秒内无崩溃、日志无输出
```

`shellcheck scripts/package-macos.sh` → 无告警（exit 0）。

### 真实 mise 上的探测命令（mise 2026.10.15 macos-arm64）

```
[0] mise registry --json
[0] mise ls --json
[0] mise config ls --json
[0] mise env --json-extended
[0] mise tasks ls --json
[0] mise outdated --json
[0] mise ls-remote node --json
```

7 个探测命令在真实 mise 上全部 exit 0，即设置页的探测列表在正常环境下应全绿。
（注：若在 zsh 里用 `mise $c` 这种不分割的写法复现，会看到
`mise ERROR no tasks defined …`——那是 shell 把 "registry --json" 当成**一个**
参数传给了 mise，mise 于是按任务名处理。App 侧传的是 `Vec<String>` 独立
argv，不受影响。）

## M5 验收项

| 项 | 结果 | 依据（inspection, file:line） |
|---|---|---|
| 设置页入口 | PASS (inspection) | `Selection::Settings` `crates/mise-hub/src/state.rs:17-18`；侧边栏「工作台」末尾导航项 `views/sidebar.rs:56-62`，选中态 id `views/sidebar.rs:151`；主区 dispatch `app.rs` `right_pane` match 分支 `Selection::Settings => self.settings_pane(cx)` |
| 主题切换 | PASS (inspection) | 头部与设置页共用 `RootView::apply_theme_setting` `app.rs`（`theme::apply_theme` + 写 `AppSettings` + 落盘），设置页 picker `views/settings.rs` appearance 段；主题名来源 `theme::theme_names` `crates/mise-hub/src/theme.rs:36-42` |
| 语言切换 | PASS (inspection) | 共用 `RootView::apply_locale_setting` `app.rs`：切 locale、写设置、落盘，并重新求值启动时烘焙进 InputState 的 placeholder（`tools.search_installed` / `discover.search`），无需重启即生效；locale 列表 `app.rs` `LOCALES` |
| mise 诊断：可执行文件路径 | PASS (inspection) | `Mise::program()` `crates/mise-core/src/mise.rs:50-53`；报告里带 `program` `crates/mise-core/src/diagnostics.rs:33-40`；UI 展示 `views/settings.rs` `settings.mise_path` |
| mise 诊断：版本 | PASS (inspection) | `Mise::version()`（`mise --version` 首段）`crates/mise-core/src/mise.rs:35-43`；失败时 `version: None`，探测仍继续（测试 `diagnostics::tests::version_failure_keeps_probes_running_with_none_version`） |
| mise 诊断：JSON 能力探测 | PASS (inspection) | 7 个只读全局探测 `diagnostics.rs` `PROBES`（registry / ls / config ls / env --json-extended / tasks ls / outdated / ls-remote）；`Mise::diagnostics()` `crates/mise-core/src/mise.rs:55-88` 每个探测**只跑一次**进程；argv 顺序断言 `diagnostics::tests::every_probe_runs_globally_without_dash_c_and_in_order`（含「无 `-C`」断言）；单点失败不影响其余探测 `a_failing_probe_carries_the_raw_error_and_others_still_run`；失败保留 mise 原始 stderr |
| 探测失败展示原始错误 | PASS (inspection) | `Probe.detail` 保存 `CoreError` 原文（`diagnostics.rs:22-31`），UI 直接渲染 `views/settings.rs` `probe_row` 的 detail 行（rule 9） |
| 诊断不阻塞 UI 线程 | PASS (inspection) | `RootView::rerun_diagnostics` `app.rs`：`std::thread::spawn` + `async_channel` + `cx.spawn` 回主线程（与既有 `load_outdated_for_selection` 同款桥接）；`diagnostics_in_flight` 防重入 |
| 应用信息（版本 / 路径） | PASS (inspection) | `mise_core::version()`、`settings_path`、`state.store_path` 经 `SettingsSnapshot` 传入 `views/settings.rs` `app_info_section`；快照在渲染前取好，渲染期不再读实体（gpui 二次 `read` 会 panic，见 `app.rs` `env_pane` 注释） |
| 更新：清单格式与解析 | PASS (inspection) | `crates/mise-core/src/update.rs` `UpdateManifest { version, page_url }`；`parse_manifest` 拒绝空版本（`empty_version_is_an_error_not_up_to_date`）与非法 JSON；`page_url` 缺省为 None |
| 更新：版本比较 | PASS (inspection) | `is_newer` 逐段数值比较（`0.10.0 > 0.9.0`、缺段补 0、剥 `v` 前缀）；预发布不越过正式版（`0.2.0-rc.1` 不新于 `0.2.0`，`0.2.0` 新于 `0.2.0-rc.1`）——测试 `is_newer_compares_numeric_segments` |
| 更新：检查流程 | PASS (inspection) | `RootView::check_for_updates` `app.rs`：URL 为空直接拒绝（`AppState::update_check_enabled` 纯守卫 + 单测 `update_check_requires_a_non_blank_url`）；`fetch_manifest`（ureq，10s 超时）只在后台线程调用；结果三态 UpToDate / UpdateAvailable / Failed（Failed 存原始错误） |
| 更新：URL 可配置并持久化 | PASS (inspection) | `AppSettings.update_manifest_url`（`#[serde(default)]`，旧 settings.json 缺字段时为空=禁用，不猜默认值）`crates/mise-core/src/settings.rs`；设置页输入框为草稿，「保存」写入并落盘 `RootView::save_update_url` |
| 更新：打开下载页 | PASS (inspection) | `crates/mise-hub/src/opener.rs`：`is_openable_url` 只放行 http/https 且拒绝空白字符（`file:` / `javascript:` / 路径全部拒绝）；URL 作为**单个 argv 参数**传递，不走 shell 字符串（`opener_passes_the_url_verbatim_as_its_own_argument`）；macOS `open` / Windows `cmd /C start "" <url>` / 其他 `xdg-open` |
| 更新：不自动下载安装 | PASS (inspection) | 界面只有「打开下载页」按钮，代码里没有任何下载/替换自身逻辑（`update.rs` 只有 GET 清单） |
| i18n 补全 | PASS (inspection) | 全量扫描 `crates/mise-hub/src`：**无用户可见的硬编码字符串**（唯一的中文常量是语言名 `中文` 本身）；终端标签页标题改走 `t!`（install / use / uninstall / set / unset / run / watch / upgrade / 备份行），规格与 flag 原样保留；删除 M1 遗留无引用键 `nav.not_available` |
| locale 键集一致 | PASS (inspection) | 新测试 `i18n::tests::locale_files_have_identical_key_sets` 比较两份 yml 的**嵌套**键路径集合（204 键，含 `settings.probe.*`）；反向验证：临时删掉 zh 的 `settings:` 段后该测试 FAILED，恢复后 PASS（测试有牙齿，非空断言） |
| 打包：macOS .app | PASS (inspection + 实跑) | `scripts/package-macos.sh`：版本从根 `Cargo.toml` 的 `[workspace.package]` 读出写入 Info.plist（`CFBundleShortVersionString`/`CFBundleVersion`）；目录结构 `Contents/{MacOS,Resources}` + Info.plist；`plutil -lint` 通过；二进制实启动不崩溃 |
| 打包：未签名说明 | PASS (inspection) | 脚本头注释与 README 明确「未签名未公证」，并给出 Gatekeeper 右键打开 / `xattr -dr com.apple.quarantine` 的绕过说明（不假装已公证） |
| 打包：Windows / Linux | PARTIAL | CI 里用 `cargo build --release --target …` 产出二进制并打包（zip / tar.gz）。**本机无法验证**（无 Windows/Linux 环境）——CI 首次实际运行前只能标注 UNVERIFIED |
| CI：push tag 出 Release | UNVERIFIED (needs a repo) | `.github/workflows/release.yml`：tag `v*` 触发，matrix 出 macos-arm64 / windows-x64 / linux-x64 三个产物，`softprops/action-gh-release` 发布。仓库目前**没有 git remote**，因此 workflow 未在 GitHub 上跑过；Linux job `continue-on-error`（`matrix.experimental`）确保 Linux 失败不阻塞 macOS/Windows 发布 |
| CI：日常检查 | UNVERIFIED (needs a repo) | `.github/workflows/ci.yml`：fmt / clippy `-D warnings` / test。同一原因未在 GitHub 上运行过；本地等价命令全部通过（见上） |
| 文档 | PASS (inspection) | `README.md`：功能表、构建运行、macOS 打包与 Gatekeeper 说明、CI 发布流程、更新清单 JSON 格式与「无 URL 即禁用」说明、三平台支持表、项目结构。`docs/superpowers/plans/2026-10-22-mise-hub-m5.md` 为本里程碑计划 |
| GUI 整体行为（设置页渲染与交互、探测徽标、更新检查三态、打开下载页、主题/语言切换即时生效） | UNVERIFIED (GUI-only) | 人工命令：`cargo run -p mise-hub`（或 `dist/mise-hub.app`）。建议步骤：① 侧边栏点「设置」；② 看诊断区是否逐条列出 7 项探测的 OK/FAIL（`ls-remote` 应带「联网」标记）；③ 「重新探测」应显示探测中→结果，且不卡界面；④ 填入一个清单 URL（如本地 `python3 -m http.server` 下的 `{"version":"99.0.0","page_url":"https://example.com"}`）→ 「检查更新」应显示新版本并出现「打开下载页」；⑤ 留空 URL 时按钮应禁用并给出说明；⑥ 切换语言/主题应即时生效且重启后保持 |

## 已知边界（未实现，非缺陷）

- 更新检查只做「版本比对 + 打开下载页」，**不**自动下载、不自动替换、不做签名校验（用户已确认的范围）。
- 打包脚本**不**签名、不公证（无证书）；CI 的 macOS 产物同样是未签名 `.app`。
- 无 Windows 安装器（msi/exe installer）与 Linux 包格式（deb/rpm），当前只有 CI 产出的压缩包。
- 诊断探测里的 `ls-remote node --json` 会联网，失败时按「联网探测失败」标注，不等于功能缺失。
- 仓库没有 git remote，CI workflow 与 Releases 页面尚未真实运行过。

## Review Gate

见 `.superpowers/sdd/m5/review-report.md`（独立 reviewer，只读，评审范围 `db62d23..HEAD`）。
