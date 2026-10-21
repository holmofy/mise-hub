# M3 验收记录 — mise-hub M3（环境变量）

Date: 2026-10-21 · Branch `feat/m2-m4` (HEAD `f63b71a`) · Baseline mise 2026.10.x

## 自动化检查（真实输出）

All commands run at repo root, 2026-10-21.

### `cargo fmt --all -- --check`

```
clean (no output, exit 0)
```

### `cargo clippy --workspace --all-targets -- -D warnings`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.15s
warning: the following packages contain code that will be rejected by a future version of Rust: block v0.1.6
note: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
```

(The `block v0.1.6` future-incompat warning is a transitive dependency
notice, not a clippy lint failure; exit code 0.)

### `cargo test --workspace`

```
test result: ok. 100 passed; 0 failed; 0 ignored; 0 measured
     (mise-core, Running unittests src/lib.rs)
test result: ok. 104 passed; 0 failed; 0 ignored; 0 measured
     (mise-hub, Running unittests src/main.rs)
test result: ok. 0 passed; 0 failed   (doctest target)
```

204 tests total, 0 failures.

### `cargo build --workspace`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.46s
warning: the following packages contain code … block v0.1.6 … (exit 0)
```

## M3 验收项

| 项 | 结果 | 依据（inspection, file:line） |
|---|---|---|
| 声明 / 生效双视图 | PASS (inspection) | 三个标签页（声明 / 生效 / 差异）：`crates/mise-hub/src/views/env.rs:355-360`，tab 分派 `env.rs:423-438`；声明页按 PAGE 分组渲染各自来源文件（组标题 + 折叠链）`-- source group` `env.rs` §1.2 → `group_by_source` `env.rs:196`；生效页（`mise env --json-extended`）另列。两个数据流分开：`load_env_for_selection` 同时装载 declarations + effective `crates/mise-hub/src/app.rs:844-869`；core 侧 `mise env --json-extended` 解析 `mise-core/src/mise.rs:225-232`、`mise set` 明文列表 `mise.rs:232-240` |
| 来源追溯（source + tool） | PASS (inspection) | 每行展示 `source` 文件与该条来源（含 tool/bin 后缀标注）：`declaration_row` 传入 source 与 `config::env_bin` 标注 `env.rs:586-…`；覆盖链展开（同一变量多来源）`toggle_env_expanded` `app.rs:1002-1009`；数据层携带 `source: PathBuf` `mise-core/src/env.rs:13-…`（`EnvDeclaration`）；来源缺失行不 panic（serde default policy，见 ledger T1 ruling） |
| PATH 分段 | PASS (inspection) | PATH 值按 `:` 分段并对每段打标签（config / tool bin / system）：`classify_path_segment` `env.rs:54`、`config_path_tokens` `env.rs:71`、`path_segment_hints` `env.rs:99`；专用 path_section UI `env.rs:1069`（声明页 `env.rs:933`）与生效页 `env.rs:1097`；分段规则与空段/整值测试 `env.rs:1488-1565`（`path_value_splits_into_tokens` / `tool_bin_segment_classified` / `config_segment_classified` / `system_segment_falls_through` / `path_hints_annotate_every_segment`） |
| 新增 / 编辑变量（含 --file 目标） | PASS (inspection) | 编辑 dialog 只预填不写入（rule 3）：`prompt_env_edit` `app.rs:1035-1080`，预填该 scope 的现值 `app.rs:1041-…`；确认后 `run_set_env` `app.rs:1170-1199`：有 source 文件 → `start_set_env_file`（`mise [-C dir] set --file <file> NAME=VALUE`，`mise-core/src/mise.rs:269-287`），否则按 page scope 走 `start_set_env`（`mise set [--global]`，`mise.rs:253-267`）；命令预览直显精确命令 `set_command_preview` `env.rs:125-146`；argv 测试 `state.rs:2343/2372/2422`、`mise.rs:752/762`；项目页无 root 拒绝执行 `state.rs:2476` |
| 删除变量（确认 + 精确文件） | PASS (inspection) | 二次确认 + 影响范围（rule 10）：`prompt_env_delete` `app.rs:1208-…`，对话框明确目标文件（`env.delete_confirm_body`）或回退 scope（`env.delete_confirm_scope`）；目标解析 `unset_target` `env.rs:169-…`（行级 source 文件优先，避免全局行在项目页静默 no-op——`env.rs:156-164` 与删除针对性测试 `env.rs:1610/1620`）；执行 `run_unset_env` `app.rs:1262-1289` → `start_unset_env_file`（`mise [-C dir] unset --file <file> NAME`，`mise-core/src/mise.rs:288-…`）或 `start_unset_env`（`mise.rs:306-319`）；argv 测试 `mise.rs:772/782`、`state.rs:2451/2488/2508`；危险按钮 Danger variant `app.rs:1241-…` |
| 差异视图（全局 / 项目 / 生效） | PASS (inspection) | 三列 diff（project / global / effective）：`diff_tab` `env.rs:1166-…`，行渲染 `diff_row_element` `env.rs:1227-…`；`compute_diff` `env.rs:248-296`：同值不差、值冲突、生效≠声明、系统变量不标，全有测试 `env.rs:1692-1765`（`diff_equal_values_do_not_differ` / `diff_project_only_value_differs` / `diff_value_conflict_flagged` / `diff_effective_vs_declared_mismatch_flagged` / `diff_system_var_only_in_effective_not_flagged` / `diff_sorted_and_deduplicated`）；项目列只收项目声明 `project_scope_declarations` `env.rs:183` + 测试 `env.rs:1638/1652` |
| 敏感值隐藏 / 显示 / 复制保护 | PASS (inspection) | UI 级 mask：默认遮蔽 `is_revealed` `env.rs:298` + 显示全部 toggle；逐行显值只更新 `env_revealed`；复制保护 `copy_env_value`：隐藏值拒绝进剪贴板并给 notice `env.copy_hidden_warn`，只允许可见值复制 `copy_allowed`；测试 `env.rs`。（更正 2026-10-21：mise 2026.10.15 的 `mise set` **确实默认打码** `redact = true` 变量为 `[redacted]`，`--no-redact` 显示原始值——原「CLI 无 redact flag」的 T1 ruling 判断有误；core 现已传 `--no-redact` 并防御性处理 `[redacted]` 占位符。） |
| 未信任配置引导（shared with M2） | PASS (inspection) | 复用 M2 已验收的 trust 路径：检测 `is_untrusted_error` `state.rs:702`（M3 env 错误也接此路径 `state.rs:659`），解析 path 供 `mise trust` `state.rs:123`；常驻横幅 `trust_banner`（M2 fix 后重定位）`app.rs:1679` & on_trust 装配 `app.rs:1847-1849`；`run_trust` 显式 `mise trust <path>` `app.rs:1455`；不自动信任 |
| GUI 整体行为（双标签页真实切换、PATH 分段渲染效果、dialog 输入/OK 流程、删除确认弹窗、复制到剪贴板后的真实粘贴、未信任目录下的 env 页表现） | UNVERIFIED (GUI-only) | 人工命令：`cargo run -p mise-hub`。建议人工步骤：全局页与项目页分别查看声明/生效/差异三个标签；设一个含多段 PATH 的值检查分段标注；新增变量（预览命令核对文件路径）→ 编辑 → 删除（确认弹窗显示目标文件）；对带敏感词的变量（如 `*_TOKEN`）验证遮蔽/显示/复制拒绝；在未信任目录下打开 env 页验证横幅与 trust 恢复。 |

## 携带的 minor（来自 M3-T1/T2/T3 review，未阻塞）

摘自 `.superpowers/sdd/2026-10-20-mise-hub-m2-m4/progress.md`（仅 M3 行）：

- T1: `looks_like_path` misses `X:/` and UNC; `env_value`/`env_declarations` global argv not asserted; `set_env_stream(global=false, root=None)` inherits cwd.
- T2/T3: forced reload can race an in-flight fetch; exotic variable names can be deleted but not edited; `project_col` guard is theoretical; module comment about re-entrant reads is stale.
- （T1 ruling 更正 2026-10-21：mise 2026.10.15 的 `mise set` **默认打码** `redact = true` 声明为 `[redacted]`，`mise set --no-redact` 显示原始值；原判断有误。现在 core 请求 `--no-redact`，UI 保持 hide/show 遮蔽，`[redacted]` 字面量一律当成「需要精确读」。）
