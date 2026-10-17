# 账号存储与 token 轮换的并发

## 当前行为

- `accounts.json` 的所有读写都经过 `accounts/store/persistence.rs` 的进程内锁：`read_store_value` 读，`mutate_store` 在锁内完成“读 → 改 → 写”，内容没变就不写盘。写盘入口不再对外暴露。
- 基于已存账号的更新一律走 `update_store_account`（只在仍是当前账号时更新用 `update_active_store_account`）：闭包拿到的是写入那一刻磁盘上的账号，而不是网络请求之前读到的副本；账号已被删除时返回“账号不存在”，不会重新创建。`add_account_to_store` 只用于来自新凭据的账号（OAuth、导入 refresh_token、保存当前 auth.json）。
- `read_store_with_active_sync` 只有在 `active_id` 需要改变时才写盘；以前每次调用都整文件重写（当前账号额度同步每 60 秒一次、每次 `get_store` 都会触发）。
- `settings.json` 的读改写由 `settings/store.rs` 的锁串行化。
- `accounts.json`、`settings.json`、`auth.json`、`config.toml` 和其他走 `write_json_file` 的文件改为原子替换（`atomic_file.rs`）：写同目录临时文件 → `sync_all` → `rename` 覆盖。目标是符号链接时写到链接指向的文件，保持 `fs::write` 的原语义。
- 已存账号的 refresh_token 只通过 `quota::refresh_stored_account_tokens` 交换，同一账号同时只有一个交换在进行：
  - 自动认证刷新、刷新全部遇 401/403 后的重试、手动“刷新配额”遇 401/403 后的重试，都传入自己看到的 access_token；拿到锁后若存储里的 access_token 已经变了，说明别的路径刚轮换过，直接用新 token，不再交换。
  - 手动“刷新 Refresh Token”传 `None`，总是交换。
  - 交换成功后立即落盘新 token，再去拉配额；以前要等配额请求返回才写，期间崩溃会丢掉唯一一份新 refresh_token。
- 自动认证刷新不再因为存储里的 `auth_status == "refreshing"` 跳过账号。以前交换过程中退出应用，这个状态会永久留在 accounts.json 里，该账号再也不会被自动刷新；进程内互斥已经取代了它的作用。
- 刷新全部的“是否正在运行”检查和置位在同一把锁内完成，手动与定时不会各起一轮。
- 导入单个 refresh_token 不再改 `codex_active_mode`。这一行原本和 `set_subscription_mode()` 成对出现，95bd67e 去掉了导入时切换 Codex 模式，设置修改却留了下来：API 模式下导入账号会把设置改成订阅模式（交换失败也一样），导致远程控制被挂起、下次启动不再恢复 API 模式。OAuth 添加账号从来不改模式。
- 批量导入 refresh_token 时，每个失败条目都保留错误原文，按它在导入文件里的序号写入错误日志（`account_import_token_error`，不记录 token），结果消息列出最常见的失败原因。以前网络错误、限流和线程 panic 都被算成“token 失效”，原因全部丢弃。
- 定时“刷新全部”改为每 60 秒检查一次是否有账号到期（2026-10-17）。以前循环按开头读到的间隔整段 `sleep`，把间隔从 1440 分钟改成 5 分钟、或重新打开定时刷新，最长要等 24 小时才生效。到期仍按每个账号上次刷新（成功或失败）的时间判断，检查变频繁不会多发请求。循环里读设置、读账号、启动刷新三类失败改用 `log_event_once`，持续失败时不会每分钟记一条。验证：只过了编译和 Clippy（线程循环没有单元测试）。
- 手动刷新与 OAuth 的失败写进错误日志（2026-10-17）：手动“刷新配额”遇 401/403 时也记 `account_usage_rejected_error`（与后台路径共用 `quota::log_usage_rejected`，“待定”里的 403 HTML 问题在最常用的路径上也能留下证据）；手动刷新的 token 交换失败记 `account_auth_manual_refresh_error`（账号前 8 位、原始错误、标记失败的原因），以前只回到界面。标记账号失败时（账号刚被删除）返回的错误同时带上交换失败的原文，不再只剩“账号不存在”。OAuth 登录失败（取消除外）记 `oauth_flow_error`，自动打开浏览器失败记 `oauth_open_browser_error`。验证：只过了编译和 Clippy。
- 启动时读不了 `settings.json`（内容损坏、带 BOM、被占用）不再让进程直接退出（2026-10-17）：恢复窗口尺寸和“开机自启时只进托盘”两步失败时记 `app_start_step_error`（`step`、`error`）后继续，用默认窗口启动，界面加载设置时显示同一个错误。以前这两步的错误交给 Tauri 的 setup，release 下 panic 直接结束进程，日志里只有一行 `app_start`。托盘创建等仍然致命的步骤，结束前也先记一条带 `"fatal": true` 的 `app_start_step_error`。验证：见文末隔离真实运行记录。
- 批量导入 refresh_token 与单个导入一致（2026-10-17）：已存在的账号被替换时保留 `created_at`、`last_used_at` 和订阅快照（批量导入不会重新拉订阅，以前到期日等会被清空，导入的账号还会全部跳到列表最前）；每个导入的账号随后执行 `sync_auth_file_if_active`。以前批量导入换出了新 refresh_token，却不更新 Codex 正在用的 `auth.json`，旧 access_token 过期后 Codex 用已作废的 refresh_token 刷新会失败；此时再点“保存当前账号”还会把作废的 token 写回 store。导入函数里永远为 `false` 的 `overwrite` 参数一并删除。验证（离线）：`re_added_account_keeps_timestamps_and_subscription_until_refreshed`；`auth.json` 同步会读写正式文件，没有单元测试。
- 登录已结束的账号（最近一次交换失败，错误含 `refresh_token_invalidated`、`invalid_grant`、`session has ended` 等，判定为 `accounts::account_login_expired`，远程控制自动关闭用的也是它）不再被后台反复交换（2026-10-17）：自动认证刷新跳过它；定时“刷新全部”既不把它算作到期，也不把它列为目标。依据：6.0.6 的错误日志里，同一个 refresh_token 已作废的账号连续 6 天每 15 分钟一次 `account_auth_auto_refresh_error`、每 60 分钟一对 `account_usage_rejected_error` 和 `account_auth_retry_refresh_error`，共 336 条。用户点“刷新所有配额”“刷新配额”“刷新 Refresh Token”仍会尝试；重新 OAuth 登录或导入会换掉这份错误状态。验证（离线）：`ended_logins_are_not_refreshed_automatically_but_other_failures_are`（日志里的原始错误文本不再到期，网络错误照常重试）、`automatic_refresh_leaves_out_ended_logins_and_manual_refresh_keeps_them`。

## OAuth 回调连接（2026-09-23）

- 现象（代码推断，未收到用户报告）：OAuth 登录时，回调服务器收到一个没有请求内容的连接（浏览器预连接）或一个无法解析的请求，整个登录流程就以“读取 OAuth 回调请求失败”结束。
- 根因（已实测）：监听 socket 设为非阻塞后，Windows 上 `accept` 得到的连接继承非阻塞模式，`set_read_timeout` 不起作用，请求行还没到时 `read` 立即返回 `WouldBlock`。本机用独立探针测得：客户端连上但不发送时，`read` 在 4.7µs 内返回 `WouldBlock`。macOS 的 BSD `accept` 语义相同（推断，未在 macOS 上实测）；Linux 不继承。
- 修复：`read_callback_request` 先把连接设回阻塞再设 5 秒读超时；单个连接读取或解析失败时回 400、记录 `oauth_callback_request_error`，然后继续等待下一个连接，不再结束登录。
- 验证：`callback_request_waits_for_a_late_request_line` 用非阻塞监听 + 客户端延迟 200ms 发送请求行；去掉修复时该测试失败（`WouldBlock`），加上修复后通过。没有做真实浏览器登录。

## 关键决策和原因

- 锁是进程内的 `Mutex`。应用有 single-instance 插件，同一时间只有一个 Codex Switch 进程写这些文件。
- 锁中毒时继续使用：锁保护的是文件，写入是原子替换，持锁线程 panic 时文件要么没动、要么已完整写入，没有需要拒绝的半成品状态。
- 重命名覆盖在 Windows 上要求目标文件没有被以“不允许删除共享”的方式打开。Rust 标准库和 libuv（Node/Electron）默认都允许，Codex 读取 auth.json/config.toml 不受影响；如果别的程序独占打开了这些文件，写入会显式失败并带上 OS 错误，而不是像以前那样截断后写一半。
- 401/403 仍然会触发轮换（`error_state_is_auth_rejected`）。是否只在 401 时轮换属于策略问题，见“待定”。

## 待定

- `/wham/usage` 返回 Cloudflare 403 HTML 时是否也会触发一次轮换：代码上会（401 和 403 都算认证失败）。与 CPA 共用 refresh_token 的账号会因此失效（见 [subscription-refresh.md](subscription-refresh.md)）。目前没有 `/wham/usage` 返回 403 HTML 的现场记录，暂不改。2026-09-23 起每次因 401/403 触发轮换都会在错误日志记一条 `account_usage_rejected_error`（`status`、`code`、`message`、`rawMessage` 前 300 字符）；出现 `status: 403` 且 `rawMessage` 是 HTML 页面时，就是改为“只在 401 时轮换”的证据。

## 验证记录

### 2026-09-23：离线

- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（281 passed、5 ignored）通过。新增测试：
  - `accounts::import_export`：失败原因按出现次数分组。
  - `atomic_file`：覆盖已有内容且不留临时文件、创建新文件、替换失败时保留原目标并清理临时文件。
  - `accounts::store::persistence`：8 个线程并发 `mutate_store_at` 后 8 次修改全部保留；内容未变时文件字节不变（未被重写）；闭包返回错误时文件不变。
  - `accounts::store::operations::mutation`：更新基于当前 token 而不是更早读到的副本；更新已删除账号返回错误且不重建；按旧版 account_id 找到账号时保留存储的 profile_id。
  - `quota::auth_refresh`：同一账号的轮换严格串行、不同账号互不阻塞；存储里残留 `refreshing` 不再阻止自动刷新。
- 构建用独立的 `CARGO_TARGET_DIR`（`src-tauri/target/fresh`）：仓库从 `ai\gpt\` 迁移后，默认 target 缓存里的 tauri 构建脚本输出还指向旧路径。

### 未验证

TODO(verify): 真实运行下的并发轮换还没有观察过。原因：需要让自动认证刷新与刷新全部/手动刷新在同一账号上同时触发，离线测试只覆盖了锁本身。触发条件：安装包含本改动的版本后，第一次长时间离线（access_token 已过期）再启动应用，两个后台线程会在同一轮里处理同一批账号。检查：数据目录 `logs/codex-switch-errors.jsonl` 在启动后 5 分钟内不出现带 `refresh_token_reused` 的 `account_auth_auto_refresh_error` 或 `account_auth_retry_refresh_error`（2026-09-23 起每次失败都会记录，不再只在标记失败时记录），账号卡片没有被标成认证失败；accounts.json 里每个账号的 `auth_status` 为 `active`。判据不成立时，从该错误记录的 `account` 前缀找到账号，对照 `quota/auth_refresh.rs::refresh_stored_account_tokens` 的 `stale_access_token` 判断是哪条路径做了第二次交换。

## 回退

revert 对应 commit 后重新构建。存储格式没有变化，新旧版本读写的是同一种 accounts.json。

损坏的 `settings.json` 不再让启动失败：2026-10-17 按 [dev-preview.md](dev-preview.md) 的做法隔离真实运行，通过。沙盒 `settings.json` 写成 `{` 后启动：进程不退出，页面加载并弹出“解析 settings.json 失败: EOF while parsing an object at line 1 column 1”，`get_settings` 返回同一个错误；沙盒 `logs/codex-switch-errors.jsonl` 有一条 `app_start_step_error`，`step` 为 `restore_main_window_state`，`error` 为同一段原文。隔离运行的主窗口是隐藏的，窗口可见这一项没有看过。
