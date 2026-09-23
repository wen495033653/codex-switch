# Token 用量统计：扫描与写库

## 当前行为（2026-09-23）

`usage_stats_get` 由前端每 30 秒静默轮询一次（窗口获得焦点时也会触发），在扫描锁内按顺序执行：

1. 打开 `usage-stats.sqlite`。补齐 `session_usage` 缺失的列：一次 `PRAGMA table_info` 读出全部列名后逐个比对（原先每列查一次，共 17 次）。
2. 一次性载入全部扫描状态。
3. `scan_codex_sessions` 逐个来源读取并解析有变化的 JSONL，为每个变化文件定下结果（indexed、duplicate、ignored、before_start、missing_attribution），只排队，不写库。
4. 开启一个事务：`write_session_scan_results` 按扫描顺序写入会话行、token 事件和扫描状态；有需要时 `recompute_existing_costs_if_needed` 重新计价；统计未定价数量；`aggregate_usage_cached` 汇总；最后提交。
5. 任何一步失败，整个事务回滚，命令返回错误，同时在 stderr 打印 `[usage_stats] 刷新 token 统计失败: ...`。本次排队的结果都不落库，下次刷新会重新处理这些文件。

`session_usage` 的 `today_*`、`days_7_*`、`days_30_*` 共 15 列不再计算、写入或读取。今天、7 天、30 天的数值一直由 `aggregate` 从 `session_token_events` 按事件时间汇总，`aggregate.rs` 里的 `events.today_*` 等读的是子查询别名，不是这些列。这些列仍保留在表里（`DEFAULT 0`），不删除也不迁移，旧版本照样能打开这个库。

## 关键决策和原因

- **先读后写。** 解析文件时不持有写锁。`record_attribution`（切换账号或 API 配置时写入归属）使用自己的连接，rusqlite 默认的 busy_timeout 是 5 秒；如果把解析也放进事务，首次全量扫描会让写锁一直持有到所有文件解析完（下面的基准里约 1 秒以上，历史更多时更久），归属写入可能超时失败。代价是变化文件的解析结果在写入前都留在内存里，首次扫描时就是全部历史的 token 事件。
- **汇总成功后才提交。** 这是 `AggregateCache` 精确性的要求（见 [module-structure.md](module-structure.md)）。如果先提交再汇总，汇总失败时库已经变了，缓存却还是旧的；下次刷新看不到文件变化，就会复用这个旧结果。旧实现中每个文件自动提交，扫描中途出错时前面文件的写入已经落库，同样存在这个问题。
- **每次刷新只提交一次。** 旧实现每个变化文件提交 3 次（会话行自动提交、token 事件一个事务、扫描状态自动提交），重新计价时每行一次 UPDATE 自动提交。首次扫描的耗时主要花在这些提交上。

## 已知边界

- 某个文件排队为 missing_attribution 之后、本次事务写入之前，如果 `record_attribution` 刚好删除了 missing_attribution 扫描状态，本次写入会把这个状态写回去；该文件下次变化时才会重新判断归属。旧实现在“查询归属”和“写入扫描状态”之间也有同样的窗口，只是更短。新会话的文件一般很快就会追加内容，所以影响有限。

## 验证记录（2026-09-23）

离线检查（`src-tauri/` 下 `cargo test`）：

- `usage_stats::tests::scan_fixture_summary_matches_recorded_baseline`：固定 fixture 覆盖全部扫描结果、两个 owner 中途切换、多开实例、长短上下文计价、无价格模型、计数器回落、重复会话 ID、7 天和 30 天边界。连续四次刷新（首次、追加后、无变化、次日）的汇总结果和最终库内容（不含文件 stamp 和不再写入的窗口列）与 `testdata/scan_baseline.json` 逐项相同。基线由 347da53 的旧实现生成，并已确认在旧实现上通过。“今天”的事件都在当前时间前一小时内，其余事件至少早 25 小时，所以结果在 UTC-11 到 UTC+11 之间与本地时区无关。
- `failed_scan_write_rolls_back_the_whole_refresh`：用 SQLite trigger 让第二个文件的扫描状态写入失败。旧实现下测试失败（第一个文件的 token 事件已被提交，数量为 2，期望 1）；新实现下什么都没提交，去掉 trigger 后下一次刷新得到正确总数。
- `opening_an_older_database_adds_the_missing_session_usage_columns`：旧表结构打开时补齐 17 列，第二次打开不再改动。

一次性基准，debug 构建，300 个会话文件 × 200 个 token_count 事件，各跑 2 轮（基准代码没有提交）：

| 场景 | 旧实现 | 新实现 |
| --- | --- | --- |
| 首次全量扫描 | 10.55 s / 10.70 s | 1.24 s / 1.59 s |
| 无变化再扫描 | 63 ms / 77 ms | 60 ms / 101 ms |
| 20 个文件追加后再扫描 | 0.85 s / 1.02 s | 0.33 s / 0.47 s |
| 打开连接 20 次 | 128 ms / 104 ms | 56 ms / 60 ms |

无变化时本来就不写库，两边在同一量级，差异属于测量波动。

TODO(verify)：尚未在真实数据上运行。触发条件：含本改动的构建第一次真实运行。升级前先截图账号卡片和 API 卡片上的 token 统计，并备份 `usage-stats.sqlite`。查看升级后同一时段（期间没有新的 Codex 会话）的卡片数值，以及 stderr 中 `[usage_stats]` 开头的行。通过判据：各卡片今天、7 天、30 天、全部的 tokens 和费用与升级前一致，且没有 `[usage_stats]` 错误行。不通过时，用备份的库对比 `session_usage`、`session_token_events` 和 `session_scan_state`，从 `scan.rs::write_session_scan_results` 查起。
