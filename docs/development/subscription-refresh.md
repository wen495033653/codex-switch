# 订阅信息刷新与用量重置次数

## 问题与根因（2026-09-14）

- 账号卡片上的套餐徽标和“到期 {date}”来自 `tokens.id_token` 的 claims：`chatgpt_plan_type`、`chatgpt_subscription_active_until`（同一 claims 里还有 `chatgpt_subscription_last_checked`）。
- 这些 claims 只在 refresh_token grant 重新签发 id_token 时才可能变化。5.4.7 及之前，“刷新配额”只用旧 access_token 请求 `/wham/usage`，仅在返回 401/403 时才刷新 token，所以套餐徽标会一直停在旧值。
- `/wham/usage` 响应里带 `plan_type` 和 `rate_limit_reset_credits {available_count, applicable_available_count}`，修改前 `normalize_usage_info` 把两者都丢弃了。`/wham/rate-limit-reset-credits` 可列出每张重置券的 `granted_at`/`expires_at`，未接入。

## 本机证据（2026-09-14，账号只记 account_id 前 8 位）

- `672f189a`（pro，claims 到期 2026-09-04 已过）：当天通过 refresh_token grant 重签 3 次（04:13、04:49、06:00 UTC），每次 access_token 都轮换并落盘，新 id_token 的 `active_until`/`last_checked` 始终停在 2026-09-04。
- `6eb799f4`（pro，claims 到期 2026-09-19 未过）：06:00 UTC 重签一次，`active_until`/`last_checked` 同样不变（仍为 09-19 / 09-06）。
- CPA（香港服务器 `cli-proxy-api`）持有的 `eb2ec034`、`a4c2d3b4`（plus）在 2026-09-13 被 CPA 重签，claims 也停在 2026-09-04；CPA 管理页“续期时间”读的就是同一个 claim，没有别的来源。
- Codex OAuth access_token 访问 `/backend-api/accounts/check/v4-2023-04-27` 返回 403；`/wham/accounts/check` 只有 `plan_type`。
- 结论：refresh_token 重签拿不到新的订阅到期日；可靠的实时套餐只有 `/wham/usage` 的 `plan_type`。

## 当前实现（5.4.9）

1. `normalize_usage_info` 保留 `plan_type` 和 `reset_credits`（兼容 API 原始字段名 `rate_limit_reset_credits` 与已存储的 `reset_credits`）。
2. Codex 会话 `token_count` 事件解析 `rate_limits.plan_type`；该来源没有重置券信息，当前账号从会话同步 usage 时沿用上一次 API 返回的 `reset_credits` 和缺失的 `plan_type`（`quota/usage_store/update.rs`）。
3. “刷新配额”、“刷新所有配额”和定时刷新都只拉配额，不换 token；只有 `/wham/usage` 返回 401/403 时才走 refresh_token 重签（与 5.4.7 相同）。需要强制重签用“查看 Refresh Token”里的“刷新 Refresh Token”。
4. 前端 `parseAuthInfo`：套餐优先取 `usage_info.plan_type`，无则回退 id_token claim；新增 `resetCredits`、`expiresAtStale`、`subscriptionLastCheckedAt`。
5. 账号卡片：`available_count > 0` 时显示“可重置 {count} 次”徽标；claims 到期时间已过而 usage 套餐仍为付费时，显示“到期时间未同步”并在 tooltip 里给出旧到期时间、当前套餐和 OpenAI 最后核对时间。

## 5.4.8 到 5.4.9 的变更原因

- 5.4.8 让“刷新配额”先重签 token，并在后台刷新发现 claims 过期（套餐不一致或付费套餐到期日已过）时每 24 小时重签一次。
- 2026-09-14 的真实重签证明重签拿不到新到期日；而 refresh_token 轮换会让与 CPA 共用同一 refresh_token 的账号在对方下次刷新时报 `refresh_token_reused`（当天 `eb2ec034`、`a4c2d3b4` 在 codex-switch 里就是这样失效的）。重签只有成本没有收益，因此 5.4.9 去掉后台自动重签，“刷新配额”恢复为只拉配额。

## 已知边界

- 到期日期仍只能来自 id_token claims，OpenAI 何时更新由其后端决定，本项目无法触发。
- 与 CPA 共用 refresh_token 的账号，任一方重签都会让另一方在下次刷新时失效；自动认证刷新（token 到期前 30 分钟）仍会重签，这是既有行为。
- 重置次数只展示 `available_count`。

## 验证记录

- 2026-09-14（5.4.8）：`cargo fmt --check`、`cargo test`（242 passed、2 ignored）、`cargo clippy -- -D warnings`、`npm run check`、Node 回归 22 passed；真实手动刷新 `672f189a` 一次，claims 不变。
- 2026-09-14（5.4.9）：真实重签 `672f189a` 与 `6eb799f4` 各一次（`CODEX_SWITCH_REAL_REFRESH_PROFILE_ID=<profile_id> cargo test real_manual_refresh_reissues_subscription_claims -- --ignored --nocapture`，5.4.8 代码），两账号 claims 均不变；随后的离线检查结果见本文件末尾。
- 5.4.9 离线检查：`cargo fmt --check`、`cargo test`（236 passed、2 ignored）、`cargo clippy -- -D warnings`、`npm run check`、Node 回归 22 passed 全部通过。未做安装版 UI 点击验证。
