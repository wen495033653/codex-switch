# 模块结构与依赖约定

新增或移动代码时按这里执行。目标是高内聚、低耦合：每个文件只管一件事，依赖方向单一。

## 先记住三条

1. 依赖只能向下：上层用下层，同层之间不互相引用，子模块不引用父模块。
2. Rust 非测试代码不写通配导入（`use super::*`、`use x::*`），用到什么名字就在文件头写什么。
3. 前端的方向是 components → hooks → utils，视图组件不直接调用后端。

## Rust 约定

- 引用兄弟模块的东西，从定义它的模块导入（`super::sibling::name`），不经过父模块转手。父模块只转出 crate 其他部分真正用到的名字。
- 只被一个模块使用的类型和常量，放进那个模块，不放“公共区”。
- 可见性按实际使用确定：模块内用的保持私有，兄弟或父模块要用的写 `pub(super)`。
- 测试模块里的 `use super::*` 不受限制。
- 定义在子模块里的 Tauri command，在 `main.rs` 里按真实路径注册（例如 `commands::account::capture_current`），所以该子模块是 `pub(crate) mod`。原因：普通的 `use` 转出带不走 `#[tauri::command]` 生成的隐藏宏。
- 只被 `#[cfg(windows)]` 或 `#[cfg(target_os = "macos")]` 代码使用的导入，自己也要带同样的 `#[cfg]`。否则在另一个平台上是未使用导入，CI 会失败。
- 读取设置值的纯判断函数放在 `settings`（例如 `settings/remote_control.rs`），不要放在使用方，否则会在 `accounts` 和 `codex_launcher` 之间造成循环依赖。

## Rust 各模块的分层

箭头左边可以使用右边，反过来不行。顶层模块之间没有循环依赖。

- **session_manager**：`session_manager.rs`（命令入口）→ `trash`（删除、恢复、清除）→ `preview`、`transfer`（导入导出）、`status`（归档）、`legacy_migration` → `catalog`（会话列表）、`trash_store` → `state_db` → `rollout`、`zip` → `codex_home`（目录布局与路径规则）→ `backup` → `model`、`util`。
- **usage_stats**：`usage_stats.rs`（命令、扫描编排）→ `scan`、`aggregate` → `pricing` → `db`、`parse`、`sources` → `model`。
- **codex_sessions**：`codex_sessions.rs`（I/O 锁、provider 解析、同步入口）→ `state_threads` → `rollouts`、`global_state` → `support`。
- **codex_launcher**：`codex_launcher.rs` 是命令层。其下 `codex_app_open` → `codex_app_watcher`、`remote_control`、`cdp`、`process_control` → `shell`；`codex_app_instances` → `desktop_install`、`instance_config`；`remote_control` → `backend_status`；`proxy_env`、`ide_snapshot` 各自独立。`codex_app_watcher` 不依赖 `codex_app_instances`，桌面兼容状态由命令层取好后传入。
- **quota → accounts**：单向依赖。`accounts::usage` 属于账号领域，`quota` 是建立在它上面的调度层，这个边界不要动。

用量统计的缓存（`aggregate` 里的 `AggregateCache`）是精确缓存，不按时间过期：只有数据库路径相同、本次扫描没有写入、今天的起点没变、且没有事件跨过 7 天或 30 天的分界时，才复用上次的结果。缓存和扫描锁放在同一个 `Mutex` 里，所以读不到扫描中途的状态。改扫描或计价时，任何会改变汇总结果的写入，都必须让 `scan_codex_sessions` 或 `recompute_existing_costs_if_needed` 返回 `true`。一次刷新的全部写入在同一个事务里完成，汇总成功后才提交，失败时整体回滚，详见 [usage-stats.md](usage-stats.md)。

## 前端

- 大页面按“控制器 + 视图区块 + 纯函数”组织。控制器管状态、后端调用和事件；视图区块只接收 props；纯函数放 `utils/`。
  - 会话管理：控制器 `components/SessionManagerPage.jsx`，视图区块在 `components/session/`，纯函数在 `utils/sessionManager.js`，跨页面保留的状态在 `hooks/useSessionManagerState.js`。
  - API 模式：控制器 `components/ApiModePage.jsx`，测试结果的展示在 `components/api/`，整理与格式化在 `utils/apiTestView.js`。
  - `App.jsx` 只做装配；Codex 多实例的状态与操作在 `hooks/useCodexAppInstances.js`。
- 视图区块自己调用 `useI18n()`，不通过 props 传 `t`；其余依赖都走 props。
- `utils/errors.js` 引用 `i18n` 属于对基础层的依赖，不算越界。
- `styles/visual-refresh.css` 是最后加载的改版层。能安全并回原文件的规则已经并回（提交 `ef77759`），剩下的并回后会改变样式或无法验证，保持原样。

## 怎么验证

- Rust：在 `src-tauri/` 下运行 `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。macOS 的代码本地编不了，以 CI 的 `Check Tauri (macos-latest)` 为准。
- 前端：`npm run check`，包含语法与敏感信息检查、i18n 检查、前端测试（`npm test`，即 `scripts/test-*.mjs`）和构建。
- 前端测试只覆盖少数组件和 hook，不覆盖样式，也不覆盖会话管理、API 模式等页面的交互。所以改样式要在真实渲染下对比改动前后的计算样式；重构页面要对比改动前后的 DOM 和后端调用（2026-09-17 拆分三个大页面时的做法，见提交 `59d1044`、`eaa61f4`、`da9a065`）。
- 从页面里拆出子组件时，用 TypeScript 检查器（`checkJs`，不加载 DOM 类型库）找未定义的名字，避免 `status`、`name` 这类浏览器全局变量掩盖漏传的 prop。
- 大改之后，按 [dev-preview.md](dev-preview.md) 在隔离环境里真实运行一遍。
