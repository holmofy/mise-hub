# M2 验收记录 — mise-hub M2（工具管理）

Date: 2026-10-21 · Branch `feat/m2-m4` · Baseline mise 2026.10.x

## 自动化检查（真实输出）

All commands run at repo root, 2026-10-21.

### `cargo fmt --all -- --check`

```
clean (no output, exit 0)
```

### `cargo clippy --workspace --all-targets -- -D warnings`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.20s
warning: the following packages contain code that will be rejected by a future version of Rust: block v0.1.6
note: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
```

(The `block v0.1.6` future-incompat warning is a transitive dependency
notice, not a clippy lint failure; exit code 0.)

### `cargo test --workspace`

```
test result: ok. 63 passed; 0 failed; 0 ignored; 0 measured
     (mise-core, Running unittests src/lib.rs)
test result: ok. 62 passed; 0 failed; 0 ignored; 0 measured
     (mise-hub, Running unittests src/main.rs)
test result: ok. 0 passed; 0 failed   (doctest target)
```

125 tests total, 0 failures.

### `cargo build --workspace`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.15s
warning: the following packages contain code … block v0.1.6 … (exit 0)
```

## M2 验收项

| 项 | 结果 | 依据（inspection, file:line） |
|---|---|---|
| 升级（全部 / 单工具，含 --bump） | PASS (inspection) | 升级走 `start_upgrade` → `mise upgrade [--bump] [--global]`：`crates/mise-hub/src/state.rs:310`；UI `run_upgrade` `crates/mise-hub/src/app.rs:697`，完成后刷新 outdated + installed（app.rs:733-747）；argv 测试 `state.rs:1373/1399/1617`；project-scoped spec（`-C`）测试 `state.rs:1617`
| 卸载 | PASS (inspection) | 二次确认对话框（rule 10）：`prompt_uninstall` `app.rs:530`（明确「卸载 <spec>，配置项保留」）；`start_uninstall` `state.rs:396`，spec 原样 `tool@version`（`state.rs:1590`）；绝不碰配置（rule 3）
| 应用版本（预览 + 写入） | PASS (inspection) | 预览 `mise use --dry-run`：`use_preview` `state.rs:357`、argv 测试 `state.rs:1502/1535`；只有 `Ready` 的预览才允许 OK（`app.rs:472-475`）；确认后 `run_use` → `start_use` → `mise use [--pin [--global]]`（`state.rs:369`，argv 测试 `state.rs:1563`）；写前备份由 app 完成：`start_use` 先调用 `backup_before_write` 将目标配置文件（项目 `mise.toml` 或全局 `~/.config/mise/config.toml`）复制到应用数据目录 `backups/`（时间戳命名，不覆盖），备份失败则不启动写入；写配置本身仍只走 `mise use`/`mise set`/`mise unset`（rule 3），解析仍交给 mise
| 每工具升级（spec §6.3） | PASS (inspection) | `run_tool_upgrade` `app.rs:751` → 单工具 `mise upgrade <name> --no-bump`（由 `run_upgrade` 的 `bump=false` 分支）；范围遵循当前 selection（与列表一致）；按钮被 `is_busy("upgrade", name)` 门控
| 深度搜索 | PASS (inspection) | 仅显式点击触发（rule 5）：`run_deep_search` `app.rs:859`；`mise search --all` 解析 `mise-core/src/search.rs:parse_search_table`（theme 表格测试在 `mise-core/src/search.rs:tests`）；失败显示原始错误 + 重试 `discover.rs:579-608`；空白查询不调用 core（本任务新增 `deep_search_query_runs` `state.rs:497`，测试 `state.rs:(trust_tests) blank_deep_search_query_does_not_run`）；deep 结果安装文案改为「安装 latest」（`discover.rs:677`，新 key `discover.install_latest`）
| 信任引导 | PASS (inspection) | 检测 `is_untrusted_error` `state.rs:504-512`（签名匹配测试 trust_tests）；检测接入 `refresh_installed`/`refresh_outdated`/`ensure_registry`/`use_preview`/`start_use`/`deep_search` 的错误路径（`state.rs:202-…`、`app.rs:161-183/234-247/409-418/503-509/877-886`）；根级常驻横幅 `trust_banner` `app.rs:1124`（含路径 + 「信任此配置」按钮）；点击 → `run_trust` `app.rs:919`：显式 `mise trust <path>`（`mise-core/src/mise.rs:207`），成功清横幅并刷新 installed / 当前范围 outdated / 失败的 registry；失败以原始错误显示于 root notice。**绝不自动信任。**
| GUI 整体行为（横幅每个页面可见、按钮点击后的真实流程） | UNVERIFIED (GUI-only) | 人工命令：`cargo run -p mise-hub`。建议人工步骤：信任状态下安装一个工具 → 查看工具页「已安装 · 未生效」 → 应用到本项目 → 卸载；再在未信任目录添加项目，确认横幅出现于所有页面且点击后操作恢复。

## 携带的 minor（来自 M2-T1/T2/T3/T4 review，未阻塞）

- T1: `OutdatedTool`/`ToolInfo` 未在 crate 根 re-export；`parse_outdated` 依赖 map 键顺序；`split_once(" (")` 路径含 " (" 时的局限。
- T2: `refresh_outdated` 阻塞 UI 线程（异步化仍未做）；加载文本在非 Ready 状态显示 0；失败测试未断言原始 stderr。`--global` 缺失问题已在 T3 落实修复（签名含 global 参数）。
- T3: Discover 页 apply 只提供全局范围（诚实路径，项目按钮隐藏）；侧边栏 badge 可能反映 tools 页范围；pending-preview 时 OK 是 no-op 而非禁用按钮；global 行的项目级 apply 回落为 global（对话框有说明）。
- T4: 三条 review 修复已在本任务落实（「安装 latest」文案、删除未用 key `discover.deep_search_note`、空白查询不触发深度搜索）；deep 结果安装会话标题沿用 `install <name>@latest`（shared）。
