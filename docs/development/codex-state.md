# Codex 当前状态（`codex_state`）的读取

## 当前行为（2026-09-23）

`accounts/api_mode.rs::get_codex_state_value` 从 Codex 的 `auth.json` 和 `config.toml` 推出当前模式（api / chatgpt / unknown）和 provider 信息。每次 `store-updated` 事件、store payload、模式检查、活跃账号配额刷新和 token 统计归属都会调用它。

- 一次调用只读 `auth.json` 一次、`config.toml` 一次（`codex_config::read_config_snapshot`）。原先读 `auth.json` 2 次、`config.toml` 最多 4 次。
- `auth.json` 不存在是正常状态（还没登录），按空内容处理，不记日志。
- 读取或解析失败时，stderr 记录 `[codex_state] 读取 auth.json 失败，按空内容处理: <原因>` 或 `[codex_state] 读取 config.toml 失败，按空配置处理: <原因>`，然后仍按空内容计算状态。原先这类错误被直接吞掉。
- `read_api_key_from_auth`、`read_api_key_from_provider_config` 走同一套读取和日志；后者读一次 `config.toml`（原先读 2 次）。

## 关键决策和原因

- 签名仍返回 `Value`，没有改成 `Result`。调用方有 10 处，分布在 events、store payload、`codex_sessions`、`remote_control`、`usage_stats`、`quota::active_usage`、`commands/account/mode` 等模块，都把它当作状态值使用。改成返回错误需要逐个决定失败时怎么办，超出这次的范围。所以先把原来被吞掉的错误记进日志。
- 状态计算拆成纯函数 `codex_state_from(auth, root_config, provider_config)`，在不碰真实 `~/.codex` 的前提下可以做单元测试。

## 验证记录（2026-09-23）

- 离线检查：`accounts::api_mode::tests` 覆盖 API 模式（auth key 或 provider bearer token、`openai_base_url` 覆盖）、chatgpt 模式和 unknown 模式；`codex_config::parse::tests::snapshot_reads_root_values_and_one_table` 覆盖一次读取后分别取根配置和表。计算逻辑与原实现逐行对应，只改了数据来源。原实现直接读真实文件，没有办法离线跑同一组输入做前后对比。
- TODO(verify)：日志分支没有真实触发过（测试不写真实 `~/.codex`）。触发条件：在隔离环境（见 [dev-preview.md](dev-preview.md)）里把沙盒 `.codex/auth.json` 写成非法 JSON 后刷新账号页。查看 stderr。通过判据：出现一行 `[codex_state] 读取 auth.json 失败，按空内容处理: 解析 auth.json 失败: ...`，界面显示为未登录状态，没有崩溃。不通过时从 `accounts/api_mode.rs::read_auth_for_state` 查起。
