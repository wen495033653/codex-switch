# 订阅信息刷新与用量重置次数

## 问题与根因（2026-09-14）

- 账号卡片上的套餐徽标和“到期 {date}”原本只来自 `tokens.id_token` 的 claims：`chatgpt_plan_type`、`chatgpt_subscription_active_until`。
- 这些 claims 是 OpenAI 签发 id_token 时写入的快照，且不保证在重新签发时更新。续费或改套餐后，卡片会一直显示旧的套餐和到期日。
- `/wham/usage` 响应里带 `plan_type` 和 `rate_limit_reset_credits {available_count, applicable_available_count}`，原先 `normalize_usage_info` 把两者都丢弃了。

## 数据来源验证（2026-09-14，账号只记 account_id 前 8 位）

### refresh_token 重签拿不到到期日

- `672f189a`（pro，claim 到期 2026-09-04 已过）：当天重签 3 次（04:13、04:49、06:00 UTC），access_token 每次都轮换并落盘，新 id_token 的 `active_until`/`last_checked` 始终停在 2026-09-04。
- `6eb799f4`（pro，claim 到期 2026-09-19 未过）：重签一次，claims 同样不变。
- 结论：订阅是否过期都一样，refresh_token grant 不会刷新订阅 claims。

### 找到权威来源 `/backend-api/subscriptions`

线索来自 [codex2api PR #670](https://github.com/james-6-23/codex2api/pull/670)（"Query /backend-api/subscriptions with Codex CLI identity first"）和 [sub2api issue #3606](https://github.com/Wei-Shaw/sub2api/issues/3606)（Codex 内部接口所需的 header）。

```
GET https://chatgpt.com/backend-api/subscriptions?account_id=<account_id>
Authorization: Bearer <access_token>
Accept: application/json
ChatGPT-Account-ID: <account_id>
OpenAI-Beta: codex-1
Originator: Codex Desktop
User-Agent: codex_cli_rs/...
```

响应包含 `plan_type`、`active_start`、`active_until`、`will_renew`、`is_delinquent`、`billing_period`、`grace_period_end_timestamp` 等。

本机五个账号实测（claim vs 接口）：

| 账号 | claim active_until | 接口 active_until | will_renew | is_delinquent |
| --- | --- | --- | --- | --- |
| `672f189a` pro | 2026-09-04 | 2026-10-10 | true | false |
| `6eb799f4` pro | 2026-09-19 | 2026-09-19 | true | false |
| `94b94237` prolite | 2026-10-14 | 2026-10-14 | true | false |
| `eb2ec034` plus | 2026-09-04 | 2026-10-04 | true | true |
| `a4c2d3b4` plus | 2026-09-04 | 2026-10-04 | true | true |

### 访问条件

- 必须带上面全部 header。缺 `Accept`、`ChatGPT-Account-ID`、`OpenAI-Beta` 或 Codex `User-Agent` 会被 Cloudflare 返回 403 HTML。
- **必须走 HTTP/1.1**。同一请求在 reqwest 默认的 HTTP/2 下稳定返回 403 HTML，`http1_only()` 下返回 200（本机探针对比验证）。`/wham/usage` 没有这个限制，保持 HTTP/2 不变。
- 偶发 403：连续快速请求多个账号时出现过一次，单独重试即成功。属于限流，不是凭据问题。
- 其它已排查且拿不到到期日的接口：`/wham/accounts/check`、`/wham/profiles/me`、`/wham/settings/user`、`/wham/config/bundle`、`auth.openai.com/oauth/userinfo`、`/backend-api/accounts/check/v4-2023-04-27`、`/backend-api/me`。

## 当前实现

1. `accounts/usage/client.rs::get_subscription` 请求上述接口（HTTP/1.1 + Codex 身份头），`accounts/usage/state/subscription.rs` 归一化为 `custom.subscription`：`active_until`、`plan_type`、`will_renew`、`is_delinquent`、`fetched_at`。缺 `active_until` 视为无效，不写入。
2. `quota/subscription.rs::refresh_account_subscription` 从 store 读取当前凭据（而不是由调用方传入，避免用到已轮换的 token），拉取成功且内容变化才写回。失败时保留上一次快照，并把 code、status、message、原始响应写入 stderr（`[subscription]` 前缀，账号只记前 8 位）。
3. 调用点：手动“刷新配额”、“刷新所有配额”与定时刷新、导入账号后的后台同步。都不额外重签 token。
4. `normalize_usage_info` 保留 `plan_type` 和 `reset_credits`；Codex 会话 `token_count` 事件解析 `rate_limits.plan_type`，该来源没有重置券信息，当前账号从会话同步时沿用上一次 API 返回的值。
5. 前端 `parseAuthInfo`：到期日优先取 `custom.subscription.active_until`，接口从未成功过才回退 id_token claim；套餐优先取 `usage_info.plan_type`，其次 `subscription.plan_type`，最后 claim。
6. 账号卡片：到期徽标 tooltip 区分“到期后自动续费”和“到期后不再续费”；`is_delinquent` 为 true 时显示“欠费”徽标；`available_count > 0` 时显示“可重置 {count} 次”。仅在接口从未成功、claim 已过期且 usage 套餐仍为付费时，才显示“到期时间未同步”，避免把已知错误的过期日期当成真实到期时间。

## HTTP 请求与错误信息（2026-09-23）

当前行为：

- `/wham/usage`、`/backend-api/subscriptions`、`auth.openai.com/oauth/token` 每次请求都新建 reqwest blocking client，不共享。
- 请求发送失败、响应解析失败时，错误信息带上 reqwest 的完整原因链，并去掉请求 URL。例如连接被拒绝时是 `error sending request: client error (Connect): tcp connect error: ...(os error 10061)`，原先只有 `error sending request for url (...)`。
- 非 2xx 响应的正文读取失败时，不再当作空正文：usage/subscription 的错误状态保留 HTTP status（401/403 仍会触发 token 刷新后重试），读取错误写入 `raw_message`；token 端点在原有的状态行后追加 `读取错误响应正文失败: ...`，所以 `auth_error_is_login_expired` 的判断不变。

关键决策和原因：

- 不共享 client。reqwest 0.13.5 只在构建 client 时读取系统代理（hyper-util 0.1.20 的 `Matcher::from_system`：代理环境变量、Windows `HKCU\...\Internet Settings` 的 `ProxyEnable`/`ProxyServer`/`ProxyOverride`、macOS 网络设置）。用户会在应用运行中开关系统代理（开机自启时代理软件也可能晚于本应用启动），共享 client 会一直沿用旧的代理路由直到重启。按代理配置做 key 缓存，要么依赖 hyper-util `Matcher` 的 `Debug` 输出（不含代理认证），要么自己读三个平台的代理设置（macOS 本地编译不了），复杂度和风险都高于收益。
- 收益本身很小：本机测得 debug 构建下 build + drop 一个 client 平均 0.39 ms（默认配置）和 0.47 ms（`http1_only`），各 50 次。共享能省下的主要是同一轮“刷新所有配额”里跨账号复用连接、少做 TCP/TLS 握手，这部分需要真实网络才能测，而且连接复用后 Cloudflare 对 `/backend-api/subscriptions` 的反应（上面记录过它对 HTTP/2 和连续请求敏感）也未知。
- URL 不进错误信息：订阅接口的 URL 带 `account_id` 查询参数，而错误状态已经有 `path` 字段。

验证记录：

- 离线检查：`accounts::usage::client` 与 `accounts::oauth_tokens` 的单元测试在 127.0.0.1 上起一次性 TCP 服务（client 显式 `no_proxy`，不出本机），覆盖正文读取失败（`Content-Length: 100` 只发 8 字节后断开）、正文可读、连接被拒绝三种情况。
- TODO(verify)：真实运行中还没见过新格式的错误信息。触发条件：下一次真实运行时配额或订阅刷新失败（例如断网或代理关闭时点“刷新配额”）。查看 stderr 的 `[subscription] ... message=` 行，以及 `accounts.json` 对应账号的 `custom.usage_error.message`。通过判据：信息在 `error sending request` 之后带有具体原因，且不含 `for url` 和账号 ID。不通过时从 `accounts/usage/client.rs::http_error_message` 查起。

## 5.4.8 到 5.4.9 的变更原因

- 5.4.8 让“刷新配额”先重签 token，并在后台发现 claims 过期时每 24 小时重签一次，目的是刷新订阅信息。
- 上述验证证明重签拿不到新到期日；而 refresh_token 轮换会让与 CPA 共用同一 refresh_token 的账号在对方下次刷新时报 `refresh_token_reused`（当天 `eb2ec034`、`a4c2d3b4` 就是这样在 codex-switch 里失效的）。
- 因此 5.4.9 去掉后台自动重签，“刷新配额”恢复为只拉配额，并改用 `/backend-api/subscriptions` 获取真实到期日。

## 已知边界

- `/backend-api/subscriptions` 是非公开接口，字段和路径可能随 OpenAI 更新变化；失败时界面退回 claim 或“到期时间未同步”，不会报错中断配额刷新。
- 与 CPA 共用 refresh_token 的账号，任一方重签都会让另一方在下次刷新时失效。自动认证刷新（token 到期前 30 分钟）仍会重签，这是既有行为。
- 重置次数只展示 `available_count`，不展示 `applicable_available_count` 和每张券的到期时间。
- 旧版本（5.4.8 及以前）写 accounts.json 时会丢弃 `custom.subscription` 字段；新旧版本交替运行时该字段会在下次刷新后重新补上。

## 验证记录（2026-09-14）

- 真实运行：对全部 5 个账号执行 `CODEX_SWITCH_REAL_SUBSCRIPTION_PROFILE_ID=<profile_id> cargo test real_subscription_endpoint_fills_renewal_date -- --ignored --nocapture`，均成功写入 `custom.subscription` 并回读确认，数值见上表。其中 `94b94237` 首次遇到偶发 403，重试成功，失败日志按设计记录了 status 和原始响应。
- 真实运行：对 `672f189a`、`6eb799f4` 执行 refresh_token 重签各一次，claims 不变（5.4.8 代码）。
- 离线检查：`cargo fmt --check`、`cargo test`（240 passed、3 ignored）、`cargo clippy --all-targets -- -D warnings`、`npm run check`、Node 回归 25 passed 全部通过。
- 未做：安装版 UI 点击验证；“刷新所有配额”“定时刷新”两条路径的真实后台观察（与手动路径共用 `refresh_account_subscription`）。
