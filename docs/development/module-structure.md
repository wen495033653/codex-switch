# 后端模块结构与依赖约定

2026-09-17 按“高内聚、低耦合”整理后的约定。新增或移动代码时按这里执行。

## 约定

- 依赖只能向下：模块只能使用比自己低的层。同层模块不互相引用，子模块不引用父模块。
- 非测试代码不使用通配导入（`use super::*`、`use x::*`、`pub(crate) use x::*`）。每个文件在头部写明自己用到的名字，越界由编译器报错。测试模块内的 `use super::*` 不受限。
- 同模块内引用兄弟模块的东西，从定义它的兄弟导入（`super::sibling::name`），不经过父模块转手。
- 父模块只转出 crate 其他部分真正用到的名字。
- 只被一个模块使用的类型、常量，放进那个模块，不放“公共区”。
- 可见性按实际使用确定：只在模块内使用的保持私有，被兄弟或父模块使用的用 `pub(super)`。
- 定义在子模块里的 Tauri command 在 `main.rs` 里按真实路径注册（例如 `commands::account::capture_current`），该子模块因此是 `pub(crate) mod`。原因：普通 `use` 转出带不走 `#[tauri::command]` 生成的隐藏宏。
- 只被 `#[cfg(windows)]` 或 `#[cfg(target_os = "macos")]` 代码使用的导入，自身也要带同样的 `#[cfg]`，否则在另一个平台上是未使用导入，CI 在 `-D warnings` 下失败。

## 各模块的分层（上层可以用下层，反之不行）

### session_manager

| 层 | 模块 |
|---|---|
| 命令入口 | `session_manager.rs`：Tauri command 与模块装配 |
| 功能 | `trash`（删除、恢复、清除、已删除会话预览）→ `preview`、`transfer`（导入导出）、`status`（归档）、`legacy_migration` |
| 目录服务 | `catalog`（会话列表）、`trash_store`（回收站记录的持久化） |
| 服务 | `state_db`（Codex 状态库）→ `rollout`（单个会话文件的读写与 id 改写）、`zip` |
| 基础 | `codex_home`（目录布局、路径规则、会话索引、全局状态）→ `backup` → `model`（多个模块共用的类型）、`util`（只放与业务无关的纯函数） |

### usage_stats

`usage_stats.rs`（command、归属记录入口、扫描编排）→ `scan`、`aggregate` → `pricing` → `db`、`parse`、`sources` → `model`。
扫描结果的取值 `SCAN_OUTCOME_*` 是数据库列的词汇，属于 `db`。
`aggregate` 里的 `AggregateCache` 是精确缓存，不是过期时间缓存：只有同时满足“数据库路径相同、本次扫描没有写入、今天的起点没变、没有事件跨过 7 天或 30 天的分界”才复用上次的结果，否则重新汇总。缓存和扫描锁放在同一个 `Mutex` 里（`USAGE_STATS_SCAN_LOCK`），因此不会读到扫描中途的状态。改动扫描或计价时，要保证任何会改变汇总结果的写入都让 `scan_codex_sessions` 或 `recompute_existing_costs_if_needed` 返回 `true`。

### codex_sessions

`codex_sessions.rs`（I/O 锁、provider 解析、同步与预检入口）→ `state_threads` → `rollouts`、`global_state` → `support`（状态文件路径、provider 日志标签、仅当文件存在才写入）。

### codex_launcher

`codex_launcher.rs` 是命令层：command 包装、代理开关后的远程控制重启编排。下面各自独立：
`codex_app_open` → `codex_app_watcher`、`remote_control`、`cdp`、`process_control` → `shell`；
`codex_app_instances` → `desktop_install`（桌面版安装位置与兼容性探测）、`instance_config`（实例 `config.toml` 的逐行合并）；
`remote_control` → `backend_status`（ChatGPT 后端环境状态的 HTTP 客户端，只依赖 `json_util`）；
`proxy_env`（Codex `.env` 代理配置）、`ide_snapshot`（含 `IdeRuntime`）。
`codex_app_watcher` 不依赖 `codex_app_instances`：桌面兼容状态由命令层取好后传入。

### 顶层

顶层模块之间没有循环依赖。`quota` 单向依赖 `accounts`（约 27 个名字）：`accounts::usage` 同时被 `accounts` 自己的 `store`、`account_builders`、`import_export` 和 `commands` 使用，属于账号领域；`quota` 是建立在其上的调度层，边界保持不变。
读取设置值的纯判断函数放在 `settings`（例如 `settings/remote_control.rs`），不要放在使用方模块里，否则会制造 `accounts` 与 `codex_launcher` 之间的循环。

## 前端

- JS 导入图没有循环，方向是 components → hooks → utils，保持这个方向。`utils/errors.js` 引用 `i18n` 是对基础层的依赖，不算越界。
- 大页面按“控制器 + 视图区块 + 纯函数”组织：
  - 会话管理：`components/SessionManagerPage.jsx` 是控制器（状态、后端调用、事件处理）；`components/session/` 下是五个只接收 props 的视图区块（`SessionFilterBar`、`SessionListPanel`、`SessionPreviewPanel`、`SessionContextMenu`、`SessionConflictDialog`）；常量与纯函数在 `utils/sessionManager.js`；跨页面保留的筛选与选中状态在 `hooks/useSessionManagerState.js`。
  - API 模式：`components/ApiModePage.jsx` 是控制器；测试结果的展示在 `components/api/`（`ApiTestDetail`、`ApiCheckModal`）；结果的整理与格式化在 `utils/apiTestView.js`。
  - `App.jsx` 只做装配。Codex 多实例的状态与打开操作在 `hooks/useCodexAppInstances.js`，3 秒轮询仍由 `App.jsx` 发起。
- 视图区块自己调用 `useI18n()`，不通过 props 传 `t`。其余依赖一律走 props，不在区块里直接调用后端。
- `styles/visual-refresh.css` 是最后加载的改版层。其中 26 条规则已并回所属文件，做法和验证方式见提交 `ef77759` 的说明。其余规则留在原处：41 条并回后会改变计算样式，5 条在可渲染的界面里没有出现过无法验证，158 条在其他文件里没有相同作用域的同名规则。
- 前端没有自动化测试。改样式前后要在真实渲染下对比计算样式，不能只看构建通过。

## 验证方式

- 每一步：`cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`（在 `src-tauri/` 下）。
- 跨平台：macOS 分支本地编不了，以 CI 的 `Check Tauri (macos-latest)` 为准。
- 前端：`npm run check`。它只证明能构建。拆分组件时另外做了行为对比（2026-09-17，提交 `59d1044`、`eaa61f4`、`da9a065`）：用假的 Tauri 后端（`window.__TAURI_INTERNALS__`）和固定时钟，把改动前后的代码分别加载进两个 iframe，执行同一段操作脚本，每一步比较 `#root.outerHTML` 和后端调用序列。结果：会话管理 53 步、API 模式 34 步、多实例 40 步，DOM 逐字节相同，后端调用相同（两个定时轮询按总次数比较）。这是离线对比，不等于在真实应用里点过一遍。
- 拆出子组件时用 TypeScript 检查器（`checkJs`，不加载 DOM 类型库）找未定义的名字，避免 `status`、`name` 这类浏览器全局变量掩盖漏传的 prop。
