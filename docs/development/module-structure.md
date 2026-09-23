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
- `config.toml` 的逐行解析与格式化（`find_root_table_index`、`root_assignment`、`table_bounds`、`format_toml_string`）只在 `codex_config` 里实现，多开实例的 `instance_config` 直接复用（2026-09-23 删除了它的逐字副本；`table_bounds` 接收完整的 `[table]` 行）。
- 读写文件、枚举或结束进程、访问网络、调用 `open::that` 的 Tauri command 写成 `async fn`，工作交给 `blocking_task::run_blocking`。原因见下方“Tauri command 的执行线程”。

## Tauri command 的执行线程（2026-09-23）

- 依据：tauri-macros 2.6.3 对不带 `async` 的 command 生成 `body_blocking`，函数在 WebView 的 IPC 回调里同步执行，这个回调在 UI 主线程上（Tauri 文档同样写明非 async command 在主线程执行）；执行期间窗口不响应。`open` 5.4.4 的 `open::that` 在 Windows 上会启动 `powershell.exe` 并等它退出。
- 做法：`blocking_task::run_blocking(action, task)` 用 `spawn_blocking` 执行 `task`，调用线程只等结果。任务 panic 或被取消时返回 `{action}任务异常…`，并记 `command_blocking_task_error`（`action`、原始 JoinError）。release 配置是 `panic = "abort"`，panic 会直接结束进程，这条分支在 release 下只剩运行时关闭时的取消。`State<'_, Arc<…>>` 先 `Arc::clone` 再移入任务。前端 `invoke` 本来就返回 Promise，`desktopApi.js` 不用改。
- 已改为 async 的 command：账号的 `capture_current`、`import_refresh_token`、`delete_account`、`switch_account`、`switch_api_mode`；Codex 的 `open_codex_app_instance`、`show_codex_app_instance`、`get_codex_app_instance_status`、`set_codex_proxy_env_enabled`、`set_codex_remote_control_enabled`、`set_codex_remote_control_account_id`、`restart_open_ides`、`discard_ide_snapshot`；通用的 `get_store`、`get_settings`、`update_settings`、`set_codex_model_instructions_enabled`、`open_data_dir`、`open_external_url`、`open_codex_config_toml`、`list_brand_voice_files`、`dismiss_update_version`。原有 async command 里的重复 JoinError 处理（账号 4 个、`get_current_codex_app_processes`、`get_codex_remote_control_status`、重启）改用同一个 helper。
- 保持同步的 command：只读内存或环境变量的 `get_app_version`、`get_data_dir`、`get_refresh_all_status`、`get_dev_log_entries`、`oauth_submit_callback`；只调剪贴板的 `copy_text`；`install_update` 安装后进程直接退出，本地无法验证换线程后的安装流程。`oauth_start`、`oauth_cancel` 用 `#[tauri::command(async)]`，本来就不在主线程。`refresh_all_quotas` 会读 `accounts.json`，属于额度模块，本次没改。
- 注意：这些命令以前在主线程上逐个执行，现在可以并发。`accounts.json` 与 `settings.json` 的读改写没有加锁；后台额度刷新和 watcher 本来就与命令并发，这不是新出现的竞争，但命令之间的并发是新的。
- 验证：`cargo test blocking_task` 覆盖任务在调用线程之外执行、错误原样返回、panic 转成错误；完整 `cargo test`、Clippy all-targets、fmt 通过。没有真实运行界面。

TODO(verify): command 移出主线程后没有在真实界面上运行过。原因：本次只做了离线检查，按要求没有启动应用。触发条件：下一次按 [dev-preview.md](dev-preview.md) 做隔离真实运行，或安装包含本改动的版本后第一次使用多开“显示窗口”。检查：隔离运行中上面列出的可安全调用的命令（`get_store`、`get_settings`、`update_settings`、`get_codex_app_instance_status`、`list_brand_voice_files`、`set_codex_proxy_env_enabled`、`set_codex_model_instructions_enabled`、`dismiss_update_version`）返回与改动前相同的结构；数据目录下 `logs/codex-switch-errors.jsonl` 没有 `command_blocking_task_error`。`show_codex_app_instance` 现在在 worker 线程调用 `SetForegroundWindow`，Windows 按进程判断前台权限，预期仍能激活窗口。通过判据：点击已运行实例的“打开”能把该 Codex 窗口带到前台，且不返回“独立 Codex 窗口激活失败”。判据不成立时，从 `codex_app_instances.rs` 的 `focus_instance_window` 继续：进程查询留在 worker 线程，只把激活窗口放回主线程执行。

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
