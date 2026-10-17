# Token 用量统计

## 当前行为（2026-09-23，2026-10-17 加入逐条账单和补价）

### 数据来源

只使用 Codex 在 rollout 文件里为每次 API 响应写下的 `token_usage_record`。根据 Codex 开源代码（`codex-api/src/sse/responses.rs` 的 `From<ResponseCompletedUsage> for TokenUsage`），它的 `usage` 就是 Responses API `response.completed.usage` 的原样字段：

- `input_tokens`：全部输入，已包含缓存命中和缓存写入。
- `cached_input_tokens`：输入中缓存命中的部分。
- `cache_write_input_tokens`：输入中写入缓存的部分。Codex 缺少该字段时按 0 处理。
- `output_tokens`：全部输出，已包含推理。
- `reasoning_output_tokens`：输出中推理的部分。
- `total_tokens`：输入 + 输出。

每条记录带有 API 的 `response_id`，以它为主键去重。分叉会话会把父会话的历史（包括父会话的记录）复制进自己的文件，文件被原地改写后也会重新读取，同一个响应因此可能出现多次，按主键只计一次。

记录本身不带模型、provider 和服务档位，这三项取记录之前最近一行 `turn_context`、`thread_settings_applied`（`thread_settings.model`、`model_provider_id`、`service_tier`）或 `session_meta` 里的值。之所以按"之前最近一行"而不是"同一个 turn_id"，是因为上下文压缩请求的记录写在新 turn 的 `turn_context` 之前。2026-09-23 核对本机 50,176 条记录：50,158 条的 turn 就是最近一个 `turn_context`，其余 18 条都是这种压缩请求。

### 读取

`scan` 对每个 rollout 文件（`sessions`、`archived_sessions`，包括多开实例）保存一个读取位置（`rollout_cursors`：偏移、偏移前 64 字节、当时的 provider / 模型 / 档位）：

- 文件大小和修改时间没变，就不打开。
- 有变化时，从上次的位置接着读，只处理完整的行；最后一行还没写完就留到下次。
- 偏移前的字节对不上（会话同步改写了 provider 行，或文件被截短），从头重读。已存的记录按 `response_id` 跳过。`usage_stats_rollout_reread` 只在开发日志窗口显示，不写日志文件：切换模式后的会话同步每次都会改写全部 rollout，这条事件随之大量出现。
- 只有 `token_usage_record`、`turn_context`、`thread_settings_applied`、`session_meta` 四种行会被解析，其余行（消息、工具输出，单行可达数 MB）只做子串查找。查找用 `memchr::memmem`（2026-10-17 起；以前逐个窗口比较）：重读 282 个、2.26 GB 的真实 rollout，从 61.6 s 降到 7.8 s（release）。
- 从没读过、且修改时间不晚于记录起点的文件不打开。这类文件里不可能有需要计入的记录，它们下次被修改时再从头读。

读不了的文件记 `usage_stats_scan_file_error`，下次刷新重试。格式不对的记录行记 `usage_stats_record_line_error`，跳过该行，继续往后读。

### 计入规则

- **起点。** 时间早于或等于 `meta.records_counted_after` 的记录不计入。新建的库以建库时间为起点；从旧版迁移来的库以旧统计最后一次扫描的时间为起点（见下文"旧统计的迁移"）。
- **归属。** 按每次响应发生的时间和当时的 provider，在 `attribution` 表里找当时选中的账号或 API 配置。会话中途切换账号或模式后，后面的响应算到新的归属下。多开实例的记录固定归到实例标记里的账号。找不到归属的记录照样存下，但不计入任何卡片，只计入提示"N 个 session 缺少 Codex Switch 归属记录"。
- **计价。** 每条记录单独计价：
  - **档位。** `service_tier` 为空或 `default` 按标准档；`priority` 或 `fast` 按快速模式（官方已把 Priority processing 改名为 Fast mode，两种写法都接受）。其他档位没有价格，计为未定价。
  - **长上下文。** 这一次请求的 `input_tokens` 超过 272,000 时，按长上下文价格计。官方页面对短上下文的说明是 "≤272K input tokens"。
  - **公式。** 未命中缓存的输入 × 输入单价 + 缓存命中 × 缓存单价 + 缓存写入 × 缓存写入单价 + 输出（含推理）× 输出单价。
  - **没有价格时。** 缺哪一项价格，就记对应的未定价原因：`missing_model_price`、`missing_tier_price`、`missing_cached_input_price`、`missing_cache_write_price`；补价时找不到会话文件的记录为 `missing_service_tier`（见"价格表更新后的补价"）。
  - **已定价的费用在写入时就固定。** 价格表以后再改，已定价记录不回溯重算；只有未定价的记录会在新价格表下补价一次。
  - **单价随记录保存。** 每条已定价记录同时存下计价用的四个单价（`input_price`、`cached_input_price`、`cache_write_price`、`output_price`，美元 / 百万 token），账单按它们列出每一项费用。

### 存储与汇总

`db::insert_record` 写一条记录时，同时把它累加到 `usage_hourly`（按小时汇总）和 `usage_totals`（全部时间汇总），两张表都按归属、模型、会话和计价标签分组。汇总（`aggregate`）时：

- "全部"读 `usage_totals`。
- 今天、7 天、30 天精确到秒：窗口内的整小时读 `usage_hourly`，窗口起点到第一个整点之间读逐条记录。"今天"从本地时间零点算起，7 天和 30 天从当前时间往前推。
- 会话数是窗口内不同 `thread_id` 的数量。`pricing_contexts` 和 `unpriced_reasons` 按记录条数计数。
- 返回给前端的 JSON 结构与之前相同。前端的计价提示新增了"快速模式"，未定价原因也新增了两种。

每次刷新先读文件（这时不写库），再开一个事务写入新记录、更新读取位置、汇总，最后提交。任何一步失败，整个事务回滚；读取位置仍停在这些记录之前，下次刷新会重新读到它们。

### 旧统计的迁移

旧版按会话统计：`session_usage` 保存会话的累计值，`session_token_events` 保存 `token_count` 的增量。第一次打开旧库时，在一个事务里完成迁移：

1. 记录起点取旧库 `stats_started_at` 和 `session_scan_state.last_scanned_at` 两者中较晚的那个。旧统计在最后一次扫描时覆盖了当时文件里的全部 `token_count`，此后发生的响应由新统计按记录计入。
2. `session_usage` 的每个会话写成一行 `usage_totals`，"全部"窗口的数值因此与旧版一致。
3. 起点之前 30 天内的 `session_token_events` 写成记录，只加到 `usage_hourly`、不加到 `usage_totals`，这样升级后今天、7 天、30 天窗口里的旧数据不会突然消失。
4. 旧版没有保存逐次请求的输入量和档位，这些旧数据统一按当前价格表的标准档、短上下文重新计价。gpt-6-astra 等以前缺价格的会话，现在也有了费用。
5. 旧表原样保留，不删除，也不再读取。

不能用旧的事件重建历史：2026-09-23 本机旧库 `session_usage` 合计 790 亿 token，事件表只有 367 亿，早期会话没有事件记录。

### 逐条账单（2026-10-17）

命令 `usage_bills_list(range, ownerType, ownerId, model, page, pageSize)`（`bills.rs`）只读库里已存的记录，不扫描文件：

- **筛选。** `range` 为 `today`、`days_7`、`days_30`、`all`，窗口起点与卡片相同。归属为 `subscription` / `api_profile` 加 ID，或 `unattributed`（没有匹配到归属的记录），不传为全部。`model` 为空字符串表示模型未知的记录，不传为全部。页码从 1 开始，每页 1 到 200 条。参数不合法时返回错误，前端记 `usage_bills_list_error`。
- **结果。** `summary` 是整个筛选结果的合计（请求数、各类 token、新增输入、费用、未定价条数）；`owners` 只按时间范围统计（选了模型也不会隐藏其他归属）；`models` 按时间范围和归属统计；`records` 按时间倒序、同一秒按 `response_id` 倒序分页。
- **每条记录。** 带 `fresh_input_tokens`（输入减缓存命中和缓存写入）、`legacy`（旧统计迁移来的按会话事件记录）和 `charges`：新增输入、缓存命中、缓存写入、输出四项，各自的 token 数、单价和费用，四项之和等于记录费用。未定价的记录，以及已定价但没能回填单价的记录，`charges` 为 `null`。
- **单价回填。** 加单价列之前存下的已定价记录，在第一次打开库时由模型和计价标签取出单价；只有按这组单价重算的费用与已存费用完全相等时才写入，所以不会显示没有按它收费的单价。结果记 `usage_stats_prices_backfilled`（`filled`、`unmatched`）；有对不上的再记 `usage_stats_price_backfill_error`（条数和前 5 条的 `responseId`、模型、标签、已存费用），这些记录只显示合计。
- **"全部"与卡片的差别。** 从旧统计迁移来的库（`has_session_statistics`），卡片的"全部"含旧版按会话累计、没有逐条记录的部分；账单只有逐条记录（加上迁移时写入的最后 30 天按事件记录），合计会少于卡片。页面在"全部"下用一行说明提示。今天、7 天、30 天与卡片一致。

前端见 [renderer.md](renderer.md) 的"逐条账单页"。

### 价格表更新后的补价（2026-10-17）

价格表改了以后，未定价的记录要按新表补价，但库里没有存服务档位，档位只在 rollout 里。做法：

1. `pricing::PRICING_UPDATED_AT` 每次改价格表时更新。`meta.unpriced_records_checked_for` 记着上次检查用的是哪一版。
2. 两者不同时（`db::pending_reprice`），这次刷新把修改时间不早于"最早一条、模型在新表里有价格的未定价记录"的 rollout 全部从头重读（标为 `missing_service_tier` 的记录不算，见第 4 步）。
3. 重读到的记录按 `response_id` 已经存在，`db::reprice_stored_record` 用 rollout 里的档位和库里存的模型、token 重新计价：能定价就写入费用和单价；仍不能定价但原因变了（例如档位没有价格），就改成新的原因。小时和全部时间汇总里的那一份跟着从旧标签移到新标签。旧统计迁移来的记录不动。
4. 重读后仍然是 `missing_model_price`、而模型已经有价格的记录，说明哪个 rollout 里都没有它（会话文件已删除，或这次读失败，读失败另有 `usage_stats_scan_file_error`）。它们改标为 `missing_service_tier`（界面显示"档位未知"），以后的价格表更新不再为它们重读。
5. 同一个事务里写 `meta`，提交后记 `usage_stats_unpriced_rechecked`：`pricingUpdatedAt`、`rereadSince`、`rereadFiles`、`repriced`、`markedTierUnknown`、`stillUnpricedWithModelPrice`。

此外，任何时候重读到一条未定价的已存记录（分叉副本、被改写的文件），都会按当前价格表走第 3 步。

本次价格表加入 gpt-6.1-sol（标准档短上下文 $2 / $0.1 / 缓存写入 $2.5 / 输出 $10，长上下文 $4 / $0.2 / $5 / $15；快速模式短上下文 $4 / $0.2 / $5 / $20，长上下文 $8 / $0.4 / $10 / $30，每百万 token）。

### 会话同步前先入库（2026-10-17）

`codex_sessions::sync_codex_sessions_to_provider_now_from` 在改写 rollout 的 provider 之前，先调用 `usage_stats::store_new_usage_records`（一次普通刷新）。原因：约 4% 的记录只能从 `session_meta` 取到 provider（2026-10-17 核对 60 个真实 rollout），改写之后才读到的话，会按切换后的模式归属。入库失败时记 `session_sync_usage_records_error`（`trigger`、原始错误、`handling: sync_continues`），同步照常进行。

代价：同步要等正在进行的统计刷新结束（同一把扫描锁）。平时一次刷新几十毫秒；切换模式后的第一次刷新要重读被改写的全部 rollout（本机 2.26 GB 约 8 s）；价格表更新后的第一次刷新本机约 10 s，升级后第一次切换可能因此多等这么久。

## 关键决策和原因

- **以 `response_id` 为唯一数据来源。** 旧版取每个会话 `token_count` 的累计最大值，会漏掉分叉和子任务会话（它们与父会话共用 session id，被当作重复），计数器清零时也会少算。2026-09-23 抽查：一个分叉会话有 560 次自己的请求、1.3 亿 token，旧版一次都没算。
- **按请求时间归属、按请求计价。** 旧版按会话开始时间归属，并按模型的上下文窗口大小整会话决定是否长上下文，两者都不准。
- **按小时汇总加边缘逐条。** 旧版每次有文件变化都要重新解析整个变化文件，并重算 30 天内的全部事件。2026-09-23 实测，Codex 正在写入时每次刷新读 63MB。
- **先读后写。** 与旧版相同：读文件时不持有写锁，避免 `record_attribution`（切换账号时写入归属，busy_timeout 5 秒）等待超时。
- **补价靠重读而不是估算。** 档位决定单价（快速模式是标准档的 2 倍），库里没有存档位，按标准档补价会低估快速模式的请求。所以只给 rollout 还在的记录补价，其余标为档位未知、不计费用。2026-10-17 用户选定按重读精确补价。
- **单价按记录保存。** 账单要列出每一项的单价；价格表以后会改，按当前表反推会给旧记录配上错误的单价。

## 已知边界

- **起点所在的那一次响应。** 旧版最后一次扫描时，如果某个响应的 `token_usage_record` 已经写下、但它的 `token_count` 还没写，这次响应两边都不会计入；时间精度按整秒，同一秒内的响应也可能出现同样的情况。每个活动会话最多涉及一次响应。
- **服务档位来自线程设置。** 取的是请求发出时线程设置里的 `service_tier`，不是服务端实际执行的档位（记录里没有这个字段）。
- **模型取自之前最近的设置行。** 压缩请求用的是它之前的模型设置。如果恰好在这次压缩前切换了模型，而切换只写进了之后的 `turn_context`，这一次会记成旧模型。
- **档位未知的记录。** 2026-10-17 本机有 30,561 条 gpt-6.1-sol 记录（178 个会话，2026-09-30 至 10-07）的 rollout 已不在 `sessions`、`archived_sessions`、多开实例和会话管理的回收区里，无法补价，标为 `missing_service_tier`，费用不计入合计（界面显示"另有 N 条未定价"）。会话文件如果以后恢复并被重读，会按第 3 步补价。
- **账单查询耗时。** 本机 14.3 万条记录（release）：今天 12 ms，7 天约 0.2 s，30 天和全部约 0.6 s，主要花在合计和归属、模型列表的全表聚合上。前端每 30 秒随统计刷新重新查询一次。

## 验证记录（2026-09-23）

离线检查（`src-tauri/` 下 `cargo test`，隔离的 USERPROFILE / HOME / APPDATA）：全部 332 个测试通过，6 个标记为 ignored。`usage_stats::tests` 共 13 个，覆盖：

- 设置跟随（包括压缩请求、档位切回默认）和未写完的行。
- 格式错误行跳过。
- 追加读取与原地改写不重复计数。
- 分叉副本去重。
- 起点之前不计入、旧文件不打开。
- 按时间和 provider 归属，多开实例归属。
- 窗口边界逐秒精确。
- 返回结构（会话数、最后使用、按模型、费用）。
- 失败整体回滚。
- 旧统计迁移只进行一次。
- 按档位和上下文计价，以及各种未定价原因。

真实数据检查（测试二进制对真实的 `~/.codex` 和多开实例目录只读，写入只落在复制出来的库）：

- **迁移。** 复制 10:00 的真实 `usage-stats.sqlite` 后运行新统计。记录起点正确取到旧库最后一次扫描的时间 `2026-09-23T10:00:50.1450223Z`。13 个归属的"全部" token 都等于旧 `session_usage` 合计加上起点之后的新记录。今天、7 天、30 天与按库内记录逐条相加的结果一致。
- **读取。** 用起点为 `2026-09-03T00:00:00Z` 的新库读全部真实文件，得到 50,212 条记录、84.3 亿 token。用 Python 独立解析同一批文件，`response_id`、token 数、模型和档位全部一致，缺失 0、多出 0。
- **读盘量**（进程 ReadTransferCount，debug 构建）：

  | 场景 | 读取量 | 耗时 |
  | --- | --- | --- |
  | 升级后第一次刷新（迁移 + 读起点后改过的文件） | 62.18 MB | 4.4 s |
  | 之后无变化 | 0.86 MB | 83 ms |
  | Codex 写入中，30 秒后 | 1.43 MB | 111 ms |
  | 旧实现，Codex 写入中，每次 | 约 63 MB | 约 0.5 s |

  起点设为 09-03 时第一次需要读 3.7 GB（230 s，debug）。只有长期没刷新过统计的库升级时才会接近这个量，而且只读一次。

### 安装版 v6.0.5（2026-09-23 18:53 升级后）

- **版本。** 运行中的 `codex-switch.exe` 为 6.0.5，PID 38844，10:53:27Z 启动；事件日志有 `app_start version=6.0.5`。
- **起点。** 复制出的库里，`meta.records_counted_after` 为 `2026-09-23T10:53:20.1466252Z`，与同库旧表 `MAX(session_scan_state.last_scanned_at)` 完全相同，即 6.0.4 退出前最后一次扫描的时间。
- **衔接。**
  - 旧统计的 1,550 个会话（790.59 亿 token）全部迁移；71,338 条近 30 天事件写入了窗口。
  - 升级后 18:53:29–18:55:07 新增 4 条官方记录，共 556,309 token，归到当前账号，标准档短上下文，费用 $1.0805。
  - 每个归属的"全部"都等于旧会话合计加新记录，没有缺少归属的记录。
  - 只有 1 个会话文件建了读取位置（起点之后只有它被修改过）。
- **读盘。**
  - 采样条件：主窗口可见、未最小化，Codex 仍在使用中（不是空闲），每 30 秒采样一次，共 10 次。
  - 结果：4.30、0.86、8.84、0.69、2.33、0.68、2.17、0.91、2.41、0.90 MB，5 分钟合计 24.09 MB。升级前约为每分钟 537 MB。
  - 采样期间没有出现每 30 秒约 63MB 或每约 63 秒约 449MB 的读取。
  - 8.84MB 那一次没有定位到来源，只出现一次。
- **错误日志。** `codex-switch-errors.jsonl` 最后修改时间仍是 2026-09-20，升级后没有任何新错误。

- **活动账号额度的缓存（`codex_session_usage`）。** 它原本也有一条 TODO(verify)，判据有两条：没有每分钟约 147MB 的读取；Codex 使用中，活动账号额度仍会更新。
  - 读取：上面的采样里没有每分钟约 147MB 的读取。
  - 更新：活动账号 `usage_info.fetched_at` 为 `2026-09-23T10:57:08.571Z`，在升级之后，与最新会话文件中带 `rate_limits` 的 `token_count` 时间完全一致，说明额度是从会话文件增量读到的。
  - 该 TODO 已删除。

结论：4 项通过判据全部满足。卡片界面上的数值由用户对照升级前的截图确认。

## 验证记录（2026-10-17，逐条账单与补价）

**离线。** `cargo test`（隔离的 USERPROFILE / HOME / APPDATA）360 个通过、5 个 ignored；fmt、clippy 通过。`usage_stats::tests` 新增 3 个：

- `the_bill_lists_each_response_with_what_it_was_charged`：时间、归属、模型筛选，未归属，分页与排序，合计，归属和模型列表的统计范围，快速模式长上下文的四项单价与费用之和，参数校验。
- `prices_are_backfilled_only_where_they_reproduce_the_stored_cost`：旧表结构的库，费用对得上的回填单价，对不上的和未定价的不回填，第二次打开不再回填。
- `records_stored_without_a_price_are_priced_from_their_rollout_once`：按 rollout 里的快速档补价（长、短上下文各一条），档位没有价格的改为 `missing_tier_price`，rollout 不在的标为 `missing_service_tier`，汇总表里不再有 `missing_model_price`，`pending_reprice` 不再为档位未知的记录重读，第二次刷新不变。
- 变异检查：去掉重读（`reread_since` 置空）、去掉档位未知标记、去掉"原因变了也移动"、去掉 `pending_reprice` 的排除条件，这条测试都失败。

**真实数据副本**（release 测试二进制；真实库用 SQLite backup API 只读复制，真实 `~/.codex` 和多开实例只读）：

- 复制 19:30 的真实库：143,401 条记录（71,338 条旧版迁移），63,322 条 gpt-6.1-sol 未定价，汇总表与逐条记录完全一致，还没有单价列。
- 第一次刷新 10.5 s（之前的逐窗口查找为 56 s）：80,042 条已定价记录全部回填单价，对不上 0 条；重读 282 个 rollout（2.26 GB），32,761 条补价（标准档短 / 长上下文、快速档短 / 长上下文都有）；30,561 条标为档位未知。逐条核对这 178 个会话：`sessions`、`archived_sessions`、多开实例、会话管理目录里都没有对应文件（同样的匹配方法对已补价的 276 个会话找到 272 个）。剩下 37 条 `missing_model_price` 是旧版迁移的 `codex-auto-review`，该模型确实没有价格。
- 刷新后：按小时汇总与逐条记录逐行相等，全部时间汇总按会话等于逐条记录加迁移的会话累计，按标签逐行相等；第二次刷新 40 ms，结果不变。
- 账单查询见"已知边界"。抽查的每一页 `charges` 之和都等于记录费用。

**隔离真实运行**（[dev-preview.md](dev-preview.md) 的做法，隐藏窗口，沙盒内放真实库副本和 6 个 rollout 副本）：

- 真实命令：`usage_stats_get`、`usage_bills_list`（全部、按归属、按模型第 2 页）结果正确；非法时间范围、每页 500 条、只有归属类型没有 ID 都被拒绝，各记一条 `usage_bills_list_error`，带原始错误。前端的驼峰参数（`ownerType`、`pageSize`）与命令参数对应。
- 账单页：导航进入，今天和全部各 50 行，合计、页脚、"全部"说明正常，点行打开详情，无错误提示，无 `console.error`。
- 沙盒日志：`usage_stats_prices_backfilled` 为 `filled: 80042, unmatched: 0`；`usage_stats_unpriced_rechecked` 字段齐全（沙盒只有 6 个 rollout，`repriced: 1559`，其余标为档位未知）。
- 运行前后正式环境的 `settings.json`、`accounts.json`、`config.toml`、`.env`、`auth.json` 大小和修改时间不变。

**没有验证的。** 安装版升级后的实际补价和界面（会在用户安装新版后第一次刷新时发生，按上面的事件核对）；切换模式时同步前入库的等待时长只按刷新耗时推算，没有实测。
