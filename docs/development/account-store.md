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

## 关键决策和原因

- 锁是进程内的 `Mutex`。应用有 single-instance 插件，同一时间只有一个 Codex Switch 进程写这些文件。
- 锁中毒时继续使用：锁保护的是文件，写入是原子替换，持锁线程 panic 时文件要么没动、要么已完整写入，没有需要拒绝的半成品状态。
- 重命名覆盖在 Windows 上要求目标文件没有被以“不允许删除共享”的方式打开。Rust 标准库和 libuv（Node/Electron）默认都允许，Codex 读取 auth.json/config.toml 不受影响；如果别的程序独占打开了这些文件，写入会显式失败并带上 OS 错误，而不是像以前那样截断后写一半。
- 401/403 仍然会触发轮换（`error_state_is_auth_rejected`）。是否只在 401 时轮换属于策略问题，见“待定”。

## 待定

- `/wham/usage` 返回 Cloudflare 403 HTML 时是否也会触发一次轮换：代码上会（401 和 403 都算认证失败）。与 CPA 共用 refresh_token 的账号会因此失效（见 [subscription-refresh.md](subscription-refresh.md)）。目前没有 `/wham/usage` 返回 403 HTML 的现场记录，暂不改；出现时看错误日志里的 status 与 raw_message 再决定。

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

TODO(verify): 真实运行下的并发轮换还没有观察过。原因：需要让自动认证刷新与刷新全部/手动刷新在同一账号上同时触发，离线测试只覆盖了锁本身。触发条件：安装包含本改动的版本后，第一次长时间离线（access_token 已过期）再启动应用，两个后台线程会在同一轮里处理同一批账号。检查：数据目录 `logs/codex-switch-errors.jsonl` 在启动后 5 分钟内不出现带 `refresh_token_reused` 的 `account_auth_auto_refresh_error`，账号卡片没有被标成认证失败；accounts.json 里每个账号的 `auth_status` 为 `active`。判据不成立时，从该错误记录的 `account` 前缀找到账号，对照 `quota/auth_refresh.rs::refresh_stored_account_tokens` 的 `stale_access_token` 判断是哪条路径做了第二次交换。

## 回退

revert 对应 commit 后重新构建。存储格式没有变化，新旧版本读写的是同一种 accounts.json。
