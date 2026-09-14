# 订阅信息刷新与用量重置次数

## 问题与根因（2026-09-14）

- 账号卡片上的套餐徽标和“到期 {date}”来自 `tokens.id_token` 的 claims：`chatgpt_plan_type`、`chatgpt_subscription_active_until`（同一 claims 里还有 `chatgpt_subscription_last_checked`）。
- 这些 claims 只在 refresh_token grant 重新签发 id_token 时才可能变化。修改前，“刷新配额”只用旧 access_token 请求 `/wham/usage`，仅在返回 401/403 时才刷新 token；“刷新所有配额”和定时刷新同样不刷新 token；自动认证刷新只在 access_token 到期前 30 分钟触发（token 有效期约 10 天）。
- 因此续费或改套餐后，卡片会继续显示旧的到期日期和套餐，直到十天左右后的下一次 token 刷新。
- `/wham/usage` 响应里带 `plan_type` 和 `rate_limit_reset_credits {available_count, applicable_available_count}`，修改前 `normalize_usage_info` 把两者都丢弃了。`/wham/rate-limit-reset-credits` 可列出每张重置券的 `granted_at`/`expires_at`，本次未接入。

## 本机证据（2026-09-14，账号只记 account_id 前 8 位）

- `672f189a`（pro）：id_token `iat=2026-09-10`，claims 仍为 `active_until=2026-09-04T12:29:43Z`、`last_checked=2026-09-04T08:31:15Z`；`/wham/usage` 实时返回 `plan_type=pro`，周窗口 `reset_at=2026-09-19`，说明账号仍是付费 pro。
- 用本次修改后的手动刷新真实重新签发一次 token（`iat=2026-09-14T04:13:54Z`，access_token 已轮换并落盘），新 id_token 的 `active_until`/`last_checked` 仍停留在 2026-09-04。即 OpenAI 在这次 refresh_token grant 中没有重新核对该账号的订阅。
- `94b94237`（prolite）与 `6eb799f4`（pro）：`last_checked` 比 `iat` 早 2 秒，说明 refresh_token grant 对这两个账号确实触发了订阅核对。
- Codex OAuth access_token 访问 `/backend-api/accounts/check/v4-2023-04-27` 返回 403 HTML；`/wham/accounts/check` 返回 200，只有 `plan_type`，没有订阅到期时间。
- 结论：可靠的实时套餐来自 `/wham/usage`；订阅到期时间只有 id_token claims 一个来源，而该 claim 在订阅过期后可能被 OpenAI 冻结，重新签发 token 也不一定更新。

## 处理方式

1. `normalize_usage_info` 保留 `plan_type` 和 `reset_credits`（兼容 API 原始字段名 `rate_limit_reset_credits` 与已存储的 `reset_credits`）。
2. Codex 会话 `token_count` 事件解析 `rate_limits.plan_type`；该来源没有重置券信息，当前账号从会话同步 usage 时沿用上一次 API 返回的 `reset_credits` 和缺失的 `plan_type`（`quota/usage_store/update.rs`）。
3. 手动“刷新配额”（`refresh_account`）改为先 refresh_token 重新签发 token，再用新 access_token 拉取配额；token 刷新失败按原逻辑标记认证错误并返回 `ok:false`。
4. “刷新所有配额”与定时刷新：每个账号配额刷新成功后，用 `usage_info.plan_type` 对照 id_token claims（`quota/subscription_claims.rs`）。claims 套餐与 usage 套餐不一致，或付费套餐的 `active_until` 已过期，即判定 claims 过期并重新签发 token；距上次 token 刷新不足 24 小时时跳过（重新签发不一定更新 claims，见上文证据，避免每轮都轮换 refresh_token）。判断依据、跳过原因和签发后的 claims 摘要写入 stderr（`[subscription-claims]` 前缀）。
5. 前端 `parseAuthInfo`：套餐优先取 `usage_info.plan_type`，无则回退 id_token claim；新增 `resetCredits`、`expiresAtStale`、`subscriptionLastCheckedAt`。
6. 账号卡片：`available_count > 0` 时显示“可重置 {count} 次”徽标（英文 `{count} resets available`）；claims 到期时间已过而 usage 套餐仍为付费时，显示“到期时间未同步”并在 tooltip 里给出旧到期时间、当前套餐和 OpenAI 最后核对时间，不再把过期日期当成真实到期时间展示。

## 已知边界

- 到期日期仍只能来自 id_token claims；到期前续费（日期延长、套餐不变）不会被后台判定为过期，需要手动点击该账号的“刷新配额”重新签发 token；OpenAI 是否在签发时重新核对订阅由其后端决定。
- 重置次数只展示 `available_count`，不展示 `applicable_available_count` 和每张券的到期时间。
- 手动刷新会轮换 refresh_token；同一 refresh_token 若也在其它设备使用，其它设备需重新导入。

## 验证记录（2026-09-14）

- `cargo fmt --check`、`cargo test`（242 passed、2 ignored）、`cargo clippy -- -D warnings`、`npm run check` 通过。
- `node --experimental-vm-modules --test scripts/test-model-instructions-confirm.mjs scripts/test-native-plugins.mjs scripts/test-settings-status.mjs scripts/test-remote-control-hints.mjs scripts/test-account-subscription.mjs` 共 22 passed；新脚本覆盖 usage 套餐优先、重置次数解析与徽标渲染、过期 claims 的“到期时间未同步”渲染。
- 真实环境：`CODEX_SWITCH_REAL_REFRESH_PROFILE_ID=<profile_id> cargo test real_manual_refresh_reissues_subscription_claims -- --ignored --nocapture` 对 `672f189a` 执行修改后的手动刷新，返回 `ok=true`，3 秒后回读 accounts.json 确认 access_token 已轮换，`usage_info.plan_type=pro`、`reset_credits={available_count:0, applicable_available_count:0}` 已落盘；id_token claims 仍为 2026-09-04（见上文证据）。该测试会轮换真实账号的 refresh_token，只在需要复核时运行。
- 未做：安装版 UI 点击验证、“刷新所有配额”对过期 claims 的真实后台重新签发观察（逻辑与手动刷新共用 `refresh_stored_account_tokens`，单元测试覆盖判定与 24 小时节流）。
