# mise-hub — Agent System Prompt

> 本文件是 AI 协作者在 mise-hub 仓库里的系统提示词。只写**硬约束与红线**。
> 背景与决策记录见 `docs/superpowers/specs/2026-10-19-mise-hub-design.md`；任务细则见 `docs/superpowers/plans/*.md`。

## Project

mise-hub 是给 [mise](https://mise.jdx.dev) 做的图形界面（Rust 2024 + GPUI，`gpui-kit 0.7.3`）。
目标用户是「已经在用或想用 mise、但记不住命令的开发者」。
核心价值不是把命令行按钮化，而是**把 mise 的可发现性还给人**。

## Architecture

两个 crate，职责不可越界：

| Crate | 职责 | 禁止 |
|---|---|---|
| `crates/mise-core` | 定位 mise、调用 CLI、解析 JSON、本地搜索、项目/设置持久化。纯 Rust | 不得出现任何 GPUI 类型 |
| `crates/mise-hub` | GPUI 桌面应用，只负责状态与渲染 | 不得直接 spawn mise，所有 mise 交互一律经过 `mise-core` |

## Hard Constraints（不可违反）

1. **mise 一律通过子进程 CLI 调用。** 不链接 mise 源码、不重实现 mise 的配置语义、不解析 mise 的内部缓存或配置格式。CLI 行为随版本变化，调用 CLI 才能保证结果与用户手敲命令永远一致。
2. **所有 mise 调用带 `-C <项目目录>`**，顺序固定为 `mise -C <dir> <subcommand> ...`；全局调用不带 `-C`。「当前项目」不依赖 App 进程自身的工作目录。
3. **写配置只走 mise 自己的命令**：`mise use` / `mise set` / `mise unset`。任何情况下都不要直接改写 `mise.toml` 或全局配置文件——由 mise 决定写哪个文件、保持格式。只读展示时可以解析 TOML 补足字段，但**绝不用于写入**。
4. **「安装」与「生效」是两个独立动作。** `mise install` = 装到机器；`mise use` = 写进配置使其生效。装完必须显示为「已安装 · 未生效」，并给出显式的「应用到本项目 / 应用到全局」按钮。绝不能让用户以为装完就能用。
5. **搜索只支持英文关键词。** 本地 registry 索引的 `description` 全是英文，不做中文标签层、不做中文用途词本地匹配。需要更广覆盖时走用户显式触发的「深度搜索」入口（`mise search --all`，联网）。界面上要说明这一点，避免用户以为搜索坏了。
6. **项目只由用户手动添加**，不自动扫描任何目录。允许添加没有 `mise.toml` 的文件夹——该项目的工具/任务/环境页显示空状态，用户执行「应用」动作时再询问是否创建 `mise.toml`。以规范化后的绝对路径为唯一标识，允许嵌套，不做去重建议。
7. **不写死 macOS。** 代码里不得出现硬编码的 macOS 路径或平台假设。探测 mise 时按顺序尝试 `PATH`、`~/.local/bin/mise`、`/opt/homebrew/bin/mise`、`/usr/local/bin/mise`；Windows / Linux 必须能编译。
8. **界面文案中英双语**，默认跟随系统语言，可切换。文案一律走 `rust_i18n::t!`，视图里不得硬编码字符串。新增文案必须同时补 `locales/en.yml` 和 `locales/zh-CN.yml`。
9. **mise 缺失 / 退出码非 0 / JSON 非法都必须进入错误状态**，不能 panic、不能卡死、不能长时间空白。原始 stderr 要完整展示，不要在界面上抽象成一句「失败了」。
10. **破坏性操作必须二次确认并展示影响范围**（卸载、改配置、删变量、移除项目）。

## Red Lines（做到这些算失败）

- **变成命令行的皮肤**——每个按钮旁边贴一条 `mise xxx` 让用户自己敲。GUI 要真的把事情做完。
- **假装安装就等于生效**——混淆 `mise install` 与 `mise use`。
- **隐藏破坏性操作**——影响范围不透明就执行。
- **启动就卡**——registry 1067 条、多项目多版本都不应让界面卡顿或长时间空白。索引后台构建并缓存，界面先出骨架再填充；列表用虚拟滚动。
- **只在开发者自己机器上跑得通**——跨平台是目标。
- **假装功能已实现**——当前 milestone 未覆盖的按钮要么不渲染，要么明确标注「未实现」（如 `tools.apply_m2_note`），不要放一个点了没反应的按钮。

## Data Sources（唯一取数方式）

| 用途 | 命令 | 输出形态 |
|---|---|---|
| 工具索引 | `mise registry --json` | JSON（约 1067 条） |
| 已安装工具 | `mise ls --json` | JSON |
| 工具详情 | `mise tool <名> --json` | JSON |
| 可选版本 | `mise ls-remote <工具> --json` | JSON |
| 可升级项 | `mise outdated --json` | JSON |
| 配置文件清单 | `mise config ls --json` | JSON |
| 生效环境变量 + 来源 | `mise env --json-extended` | JSON |
| 任务清单 | `mise tasks ls --json` | JSON |
| 声明式环境变量 | `mise set` | **纯文本，无 JSON** |
| 模糊搜索 | `mise search <名>` | **纯文本表格，无 JSON** |
| 安装 / 卸载 / 升级 / 应用 | `mise install` / `uninstall` / `upgrade` / `use` | 流式文本 |

`mise search` 和 `mise set` 没有 JSON 输出。发现页的默认搜索走本地 registry 索引（不联网、瞬时响应），`mise search --all` 只作为用户显式触发的「深度搜索」。

## Coding Conventions

- **测试先行**：每个新行为先写一个会失败的测试，确认它失败，再实现，再确认通过。测试放在同文件的 `#[cfg(test)] mod tests`。
- **serde 默认值**：mise 的 JSON 会缺字段。`description` / `aliases` / `bins` / `backends` 必须有 `#[serde(default)]`；`source` 缺失时归类为 `Global`，不能 panic。
- **工具名原样传递**：带后端前缀的名字（`npm:prettier`、`cargo:ripgrep`）在搜索、安装、参数拼接时不得被拆分、改写或改大小写。
- **参数不要拼字符串**：`mise` 的参数用 `Vec<String>` 原样传递（`-C <dir>` 的 dir 可能是带空格的路径）。
- **JSON 解析失败 = 错误状态**，不是 panic，也不是静默返回空列表。
- **UI 线程不做阻塞 IO**：所有 mise 调用走异步任务，结果回到 UI 线程；同一工具的重复请求要合并（用户快速切换列表项时不应重复起进程）。
- **写配置前先备份**原文件。
- **trust 机制**：mise 要求配置被信任才能执行，检测到未信任状态要引导用户 `mise trust`，不能静默失败。
- **版本兼容**：不按版本号一刀切，而是启动时逐个探测本项目依赖的 JSON 命令是否可用；不可用的功能降级并在界面说明，同时提示升级 mise。已验证基线：mise `2026.10.x`。
- **注释语言**：rustdoc 与代码注释用英文；面向用户的界面文案走 i18n。

## Task Decomposition（复杂需求的并行拆分）

### When to Split

只有同时满足以下三条才拆：

1. 需求能切成 **3 个以上**边界清晰的工作单元（按文件/接口划分，不是按「写代码 / 写测试」这种工序划分）；
2. 每个单元**能独立验证**——完成后有一条命令能单独跑通并判定成败；
3. 单元之间**不需要实时协商**——上游接口在开工前就已经定稿。

以下情况不要拆：改动集中在同一个文件；两个单元必然写同一个文件；上游接口还没定（先把接口任务做完再并行）。

### Decomposition Rules

1. **接口先行**：第一个任务只定义类型、签名、模块声明，不写实现。等它跑通并提交后，其余任务才能并行开工——否则下游会对着没定稿的接口写代码。
2. **文件所有权互斥**：每个子任务有明确的文件清单（可写 / 只读 / 禁止碰）。两个子任务的「可写」清单不得相交。
3. **每个子任务独立走完一轮 TDD**：先写会失败的测试 → 确认失败 → 实现 → 确认通过 → 记录 RED/GREEN 的实际输出。不得把测试和实现拆给两个子任务。
4. **每个子任务独立可提交**：完成后工作区能编译、`cargo test` 全绿。
5. **子任务不碰 git**：子代理只写代码和报告，**不执行 `git add` / `git commit`**。提交由协调者统一做。

### Launching Subagents

- 用 `subagent` 工具的 `background: true` 并行派发；派发前把接口任务跑通，派发后等全部返回再合并。
- 子代理 prompt 必须包含：任务目标、**可写文件清单 / 只读文件清单 / 禁止修改的文件**、要产出的接口签名、验收命令、相关 spec 段落原文、报告格式。
- 报告格式沿用 `.superpowers/sdd/<plan>/task-N-report.md`：`Status` / `Files` / `TDD Evidence`（RED 与 GREEN 的真实命令输出）/ `Self-Review` / `Concerns`。
- **cargo 构建锁**：并行子代理共用同一个 `target/` 会互相阻塞。要么给每个子代理单独的 `CARGO_TARGET_DIR`，要么让子代理只写代码、由协调者统一跑测试。

### Coordinator's Job

- 串行完成接口任务 → 并行派发其余任务 → 汇总报告 → 统一跑 `cargo fmt --all` 和 `cargo test --workspace` → 按依赖顺序逐个提交。
- 逐份审阅子代理报告，不照单全收。报告里的 `Concerns`（尤其是「计划里的参考实现是错的」）要当场裁决并写进最终提交信息。
- 合并后自己再跑一遍完整测试，确认没有子代理顺手改了别人的文件。
- 提交之后**必须走 Review Gate**（见下节）：生成 review package，另起一个全新 subagent 审。
- 并行任务各自独立走一遍 Review Gate，review 也可并行派发。

### Anti-Patterns

- 两个子任务写同一个文件 → 合并时冲突、互相覆盖。
- 子代理擅自改接口 → 下游任务的编译全部断裂。
- 子代理自己 commit → 提交历史错乱，`git rebase` 才能收拾。
- 把「实现」和「补测试」分给不同子任务 → 违背 TDD，测试写出来永远对不上实现。
- 并行做同 crate 的任务却共用 `target/` → 表面并行，实际串行等待文件锁，还可能读到半成品。

## Review Gate（每个任务完成后必须过审）

### The Rule

- 每个任务提交后，**另外启动一个全新的 subagent** 做 review。实现者不审自己的代码，reviewer 也不能是实现那个任务的 agent。
- **必须是新会话**——不要给 reviewer 传实现者的 `sessionID`。带着实现时的思路去审，review 就退化成了复述实现理由。
- Reviewer **只读不改**：发现的问题写进报告，由修复轮处理。
- Review 没过的任务不算完成。计划里的验收命令跑过了不算数，reviewer 点头才算。

### Review Package（发给 reviewer 的输入）

在 `.superpowers/sdd/<plan>/` 下生成 `review-<base>..<head>.diff`，`<base>` 是本任务**第一个**提交之前的那个 commit（这样 reviewer 看到的是整个任务的改动，而不是最后一次提交）：

```
# Review package: <base>..HEAD

## Commits
<git log --oneline <base>..<head>>

## Files changed
<git diff --stat <base>..<head>>

## Diff
<git diff <base>..<head>>
```

reviewer 的 prompt 里必须同时给：review package 的路径、计划文档中该任务的 `Interfaces` 段落、本文档的 `Hard Constraints` 与 `Red Lines` 两节、报告格式。

### Review Dimensions（reviewer 必须逐项覆盖）

1. **红线**——有没有出现「命令行的皮肤」「假装安装等于生效」「隐藏破坏性操作」「写死 macOS 路径」「假装功能已实现」。
2. **硬约束**——mise 是否全部走 CLI；`-C` 顺序是否正确；写配置是否只走 `use`/`set`/`unset`；工具名与参数是否原样传递；文案是否走 `t!` 且两种 locale 都补齐了。
3. **测试质量**——有没有 RED→GREEN 证据；测试是否**真的能失败**（恒真断言、只测 happy path 都要点名）；serde 缺字段、错误路径、边界值是否覆盖。
4. **越界**——有没有改不属于本任务的文件；有没有擅自改接口（会破坏下游任务）；有没有夹带无关重构。
5. **计划符合度**——实现是否与 `Interfaces` 一致；偏离之处理由是否成立（例如计划里的参考实现本身是错的）。
6. **回归风险**——`cargo build --workspace` / `cargo test --workspace` 是否全绿，`cargo fmt --all -- --check` 是否干净，有没有破坏既有测试。

### Fix Loop

- Reviewer 报告分两类：**blocker**（违反硬约束/红线、测试没过、接口越界）与 **minor**（可延后的改进）。
- 有 blocker → 起一个**新的** subagent 修，不要让 reviewer 自己去改。修完再起一个**新的** reviewer 复审。**最多 5 轮**（对齐既有 `fix round 1/5` 约定）。
- Blocker 清零才算任务完成；minor 记进 progress ledger 的 deferred 列表，不阻塞。
- 并行任务的 review 也可以并行派发（各自独立 review package，互不干扰）。

### Ledger

每个任务在 `.superpowers/sdd/<plan>/progress.md` 记三行：

```
Task N: complete (commits <a>..<b>, review clean)
Task N: fix round 1/5 (1 addressed, 0 open; commits <a>..<b>)
Task N: minor (deferred): <一句话>
```

## Workflow

- **每个任务结束提交一次**，Conventional Commits：`feat(core):` / `feat(ui):` / `fix(core):` / `fix(ui):` / `chore:` / `docs:`。
- **只改与任务相关的文件**，不夹带无关重构。
- 计划文档里的代码是**参考实现，不是逐字照抄的义务**。发现参考实现有错时（例如计划里写的 `mise version()` 实际会失败），改成正确实现，并在报告里说明。
- 实现前先读 `docs/superpowers/plans/` 里对应任务的 Interfaces 段落；要偏离计划，先说明理由。

## Commands

```bash
cargo build --workspace
cargo test -p mise-core
cargo test -p mise-hub
cargo run -p mise-hub
cargo fmt --all && cargo clippy --workspace --all-targets
```

## Current Status（2026-10-20）

- 分支 `feat/m0-m1`；M0 完成，M1 进行中（Task 11 工具页实现中）。
- 已完成：workspace 骨架、runner/locator、registry + 本地搜索、已安装工具解析与范围归类、项目持久化、流式安装事件、Mise 门面、UI 骨架与侧边栏、主题与双语、mise 检测与引导页。
- 剩余：Task 11 工具页 → 12 发现页 → 13 版本选择/安装/终端面板 → 14 空状态与端到端验收。
- 后续里程碑：M2 工具管理、M3 环境变量、M4 任务、M5 收尾。
