# M4 验收记录 — mise-hub M4（任务）

Date: 2026-10-21 · Branch `feat/m2-m4` (HEAD `be11fc5`) · Baseline mise 2026.10.x

## 自动化检查（真实输出）

All commands run at repo root, 2026-10-21.

### `cargo fmt --all -- --check`

```
clean (no output, exit 0)
```

### `cargo clippy --workspace --all-targets -- -D warnings`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s
warning: the following packages contain code that will be rejected by a future version of Rust: block v0.1.6
note: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
```

(The `block v0.1.6` future-incompat warning is a transitive dependency
notice, not a clippy lint failure; exit code 0.)

### `cargo test --workspace`

```
     Running unittests src/lib.rs (target/debug/deps/mise_core-91773372d653dc7f)
test result: ok. 122 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src/main.rs (target/debug/deps/mise_hub-ed88d48f46e57a5b)
test result: ok. 121 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

243 tests total, 0 failures (M0 record baseline was 59; M2 record 204;
M4 adds +39 tasks-related tests including the T2/T3 fix round).

### `cargo build --workspace`

```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.17s
warning: the following packages contain code … block v0.1.6 … (exit 0)
```

## M4 验收项

| 项 | 结果 | 依据（inspection, file:line） |
|---|---|---|
| 任务发现（`mise [-C dir] tasks ls --json`） | PASS (inspection) | core 数据层 `Mise::tasks` `crates/mise-core/src/mise.rs:210-213`（argv 测试 `mise.rs:924-943`，非零退出→`CommandFailed` `mise.rs:974-986`）；解析 `parse_tasks` `crates/mise-core/src/tasks.rs:74-92`，缺字段 serde default `tasks.rs:48-71`、缺字段测试 `tasks.rs:241-253`、非法 JSON→错误不 panic `tasks.rs:256-259`；UI 加载 `load_tasks_for_selection` `crates/mise-hub/src/app.rs:373-427`（先 Loading 再填充，慢列表不读作「无任务」）；列表行渲染名称/描述/来源/依赖计数 `crates/mise-hub/src/views/tasks.rs:370-…`。**PARTIAL 子项 — 文件任务**：`Task.file` 已解析（`tasks.rs:30`、`file` serde 字段 `tasks.rs:70`，测试 `tasks.rs:237/252`），但 UI 没有为 file 任务加区分标记——列表与详情只显示 `source` 路径或「无来源文件」（`source_display` `views/tasks.rs:156-158`，测试 `tasks.rs:950-954`）。mise 的 file 任务名称本身（如带前后缀/冒号）原样返回（`mise.rs:208-210`，测试 `mise.rs:928-930` 断言 `test:unit` 原样存活），所以"发现"不漏任务，但用户在界面上看不出某行是文件任务。见下方 PARTIAL 说明 |
| 隐藏任务过滤 | PASS (inspection) | 默认隐藏 `hide:true` 任务，toggle 时显示：过滤谓词 `views/tasks.rs:117`（`.filter(\|t\| show_hidden \|\| !t.hide)`），toggle 与 i18n 键 `tasks.hide_hidden` `views/tasks.rs:230-…`；`show_hidden` 状态经 snapshot 传入 `app.rs:491`；`parse_hides_and_globals` `tasks.rs:267-271` |
| 依赖关系呈现（执行顺序） | PASS (inspection) | 执行顺序 = depends 链 → 任务本体 → depends_post 尾迹：`dependency_order` `crates/mise-core/src/tasks.rs:121-159`，post 尾迹覆盖前链节点的 post deps `tasks.rs:146-157`（fix round 1 补齐）；排序测试 `tasks.rs:338-407`（post 在后 `order_post_depends_run_after_task:338`、前链 post 不漏 `:355`、尾迹递归 `:372`、线性链 `:390`、菱形去重 `:397`）；UI 执行链渲染 `views/tasks.rs:597-…`、`execution_chain` `views/tasks.rs:97-…` |
| 缺失依赖 | PASS (inspection) | 缺失依赖保留在顺序中并打标：`missing` 标志 `views/tasks.rs:41`、`missing: !names.contains(name)` `views/tasks.rs:104`、标签 `tasks.chain_missing` `views/tasks.rs:54-56`、渲染 `views/tasks.rs:663-…`（着色 + 标注）；core 保留顺序 `order_missing_dependency_is_kept` `tasks.rs:410-413`；UI 测试 `chain_marks_missing_dependencies` `views/tasks.rs:849-…` |
| 环（cycle） | PASS (inspection) | `is_in_cycle` DFS 自达检测 `views/tasks.rs:65-…`（含 depends_post 回边，`views/tasks.rs:62-64` 注释），测试 `is_in_cycle_detects_self_and_shared_loops:912`、`is_in_cycle_false_for_trees:922`、只指向环者不算成员 `:917-918`；环任务详情显示整段警告替代执行链 `views/tasks.rs:431`、`597-614`（`tasks.cycle_warn` / `tasks.cycle_hint`）；core 层环终止不 panic、成员记录一次 `order_cycle_terminates_and_records_once` `tasks.rs:422-433`、自环 `:436-439` |
| 运行（run） | PASS (inspection) | 按钮 → `run_task` `views/tasks.rs:515-…` → `App::run_task` `app.rs:432-477` → `start_task_run` `state.rs:628-…` → `run_task_stream` = `mise [-C dir] run <task>`，任务名原样传递 `crates/mise-core/src/mise.rs:218-232`，argv 测试 `run_task_stream_argv_keeps_task_name_verbatim` `mise.rs:961-…`（`test:unit` 不拆分）；流式输出进底部终端面板 `app.rs:428-…`；无 root（全局页）不带 `-C` `run_and_watch_argv_without_project` `mise.rs:974-…` |
| 监听（watch） | PASS (inspection)（**无停止 API — 显式诚实说明**） | `watch_task_stream` = `mise [-C dir] watch <task>` `mise.rs:224-232`，argv 测试 `mise.rs:955-…`；防重入：watch 运行中禁用再点击 `views/tasks.rs:527-529`（fix round 1，`.disabled(run_busy \|\| watch_running)`）+ 纯守卫 `should_start_task_run` `state.rs:677-678`；会话记账 `task_watch_sessions` `state.rs:173-176/644-646`，退出清理 `finish_task_watch` `state.rs:664-667`（不误删新 watch）。**诚实说明：没有停止按钮。**核心 streamer `spawn_stream` 没有暴露 kill/终止句柄（`mise-core/src/runner.rs`），面板只有常驻「监听中」状态与文字说明（`tasks.watch_stop_note` `views/tasks.rs:548-556`）：watch 只随进程退出或应用退出结束。要加真正的停止需要 `StreamingRunner` 增加 kill API——这是 core 侧改动，记为 deferred minor（见下） |
| 日志（保存 / 复制 / 清除） | PASS (inspection) | 三个按钮在终端面板 `views/terminal.rs:102-128`：保存 `save_terminal_log` `app.rs:1463-1484`（rfd 存盘对话框 → 整段会话文本写入，错误进 notice）；复制 `copy_terminal` `app.rs:1487-1491`（剪贴板）；清除 `clear_active` `state.rs:981-…`（清行保留标签页，tooltip `terminal.clear_keeps_tab`）；文本源 `active_text` `state.rs:987` |
| 失败状态（退出码 + 原始 stderr） | PASS (inspection) | 退出码与 stderr 捕获于 runner `crates/mise-core/src/runner.rs:18/38`；非零退出标记失败 `non_zero_exit_marks_failure` `state.rs:1165`；失败 run 以 `Failed(code)` 落地且 stderr 原样展示（rule 9）`app.rs:430-…`、spawn 失败也进面板而非静默 `app.rs:445-451/903/946`（rule 9 注释：failed run must not be a silent nothing） |
| 未信任配置引导（shared with M2/M3） | PASS (inspection) | 复用已验收 trust 路径：错误检测 `is_untrusted_error` `state.rs:702`，`run_trust` 显式 `mise trust <path>` `app.rs:1455`，常驻横幅 `trust_banner` `app.rs:1679`；tasks 数据层对未信任装成 `CommandFailed`（`mise.rs:974-986`）进入同一错误展示路径，不静默失败 |
| GUI 整体行为（真实列表加载、依赖树渲染、run/watch 实际流式输出、日志按钮真实落盘/粘贴、失败任务的面板呈现、未信任目录下任务页表现） | UNVERIFIED (GUI-only) | 人工命令：`cargo run -p mise-hub`。建议人工步骤：项目页查看 `mise tasks ls` 与某个文件任务的行（核对 PARTIAL 子项的观感）；展开一条有依赖的任务检查执行顺序标注与缺失/环标记；点运行查看底部面板流式输出；对会失败的脚本再运行，确认失败颜色与 stderr 原文；点一次 watch 观察「监听中」与不可再点；保存日志 → 重新打开核对内容；清除 → 标签仍在。 |

### 关于文件任务的 PARTIAL 说明

("任务发现"验收项总评 PASS，子项 **文件任务区分 = PARTIAL**.) mise 把文件任务
作为 `tasks ls --json` 的普通行返回（`(pre)task` 风格的名字原样保留，解析层不拆
不改），发现层面没有遗漏。但界面上 file 任务和普通配置任务长得一样：`Task.file`
字段已解析并存进结构体（`tasks.rs:30`），没有任何视图引用它（`grep "\.file\b"
views/tasks.rs` 仅命中测试构造），用户无法在列表里辨认「这是文件任务」。同时
AGENTS.md 红线禁止我们自动扫描/改写——所以正确的补法是在列表/详情对
`task.file.is_some()` 行加一个来源标记（i18n 两份 locale 同步），属于小的 UI
跟进项，记入 deferred minor。

## 携带的 minor（M4 未记录到 ledger 时的 review 出处）

`progress.md` 尚无 M4 行（截至 HEAD `be11fc5`）。以下摘自 M4 任务报告与 review
package 中可见的 Concerns（`.superpowers/sdd/2026-10-20-mise-hub-m2-m4/m4-t1-report.md`、
`m4-t2-t3-report.md`，及对应的 review 包头部记录的 fix 轮次）：

- **Watch 无停止 API（T2/T3 Concerns #1，已在上表诚实说明）**：`StreamingRunner`
  需要 kill/terminate 句柄（core 侧改动，超出该任务可写范围）；当前只有
  「监听中 + 文字说明」。
- **文件任务不可区分**（本次验收发现的 PARTIAL，见上节）：UI 需为
  `task.file.is_some()` 加标记。
- **环内拓扑序无定义（T1 Concerns）**：环成员在执行顺序里位置靠前无语义保证；
  core 只保证不 panic、不重复，UI 已按 T2/T3 补了显式环警告。
- **流式 run/watch 无同步退出码（T1 Concerns）**：失败语义由面板层观测
  （`Failed(code)`），与既有 `install_stream` 一致。
- **T2/T3 Concerns #2（记录性）**：`TaskGraph` 结构在 T1 之后重构（`edges` +
  `post_edges`），两个 T1 测试改名/重写；grep 确认无其他调用方，破坏可控。
