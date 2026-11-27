# 会话管理与会话同步的文件读写

范围：`src-tauri/src/session_manager/`（归档、永久删除、导入导出、预览、旧版数据迁移），以及 `src-tauri/src/codex_sessions/` 中读写 rollout、`state_5.sqlite`、`.codex-global-state.json` 的部分。Codex 的结束与重启流程见 [codex-restart.md](codex-restart.md)。读写会话数据仍须先拿 `codex_sessions::lock_codex_session_io(...)`（见 CONTRIBUTING.md）。

## 当前行为

### 目录与压缩存储（`catalog.rs` / `codex_home.rs`）

- `thread/list` 仍是目录唯一数据源；分别列出未归档与已归档，并跟随各自的 `nextCursor`。Codex 将旧 rollout 压缩为 `.jsonl.zst` 后，RPC 和 state DB 的路径仍可能是原来的 `.jsonl`。只检查该路径本身会让压缩历史从目录消失，并非分页只读取了 50 条。
- 目录、预览和 state DB 元数据查找在同一会话 I/O 锁规则下识别两种存储。列表的 `relative_path` 保留逻辑 `.jsonl`；`source_path` 与大小对应实际文件。原始文件优先；两种文件均不存在时仍返回该行的错误，不把失效索引或 app-server 未返回的会话加入目录。
- 会话页照常显示扫描返回的可读会话。遇到未读出的会话，用「已列出」代替「总计」，并在筛选栏下方显示可读数和失败数；重复会话等扫描提示单独统计，具体原因可展开。整次刷新失败不覆盖原列表，明确显示当前为上一次结果（或从未获取列表）。该状态和列表一同跨页面保留；新扫描成功后替换旧状态，已被更新请求取代的返回不会覆盖新列表。

### 归档 / 取消归档（`status.rs`）

同一把 I/O 锁内分三个阶段：

1. 文件移动，全部可逆：普通移动用 `rename`；“修改 ID”先写一份改了 ID 的副本，原文件保留；“覆盖”先把目标改名为备份。计划阶段认为空闲、执行时却已存在的目标不会被替换，记为失败。
2. 一个 state DB 事务（`state_db::apply_status_moves_to_state_db`）：先删除被覆盖会话的行及其子表行，再更新每个移动行的 `archived`、`archived_at`、`rollout_path`；“修改 ID”时同一事务内把子表（`thread_dynamic_tools`、`thread_goals`、`thread_spawn_edges`、`stage1_outputs`、`agent_job_items`）引用的 thread id 一起改掉。整批只备份一次数据库；这些会话在 `threads` 和子表里都没有行时，不备份也不开事务。
3. 事务成功后才做不可逆清理：删除“修改 ID”的原文件和覆盖备份、清理空目录，再清理 `.codex-global-state.json` 里被覆盖会话的 id。

- DB 失败（打不开、缺 `threads` 表或缺 `id/archived/archived_at/rollout_path` 列、SQLite 报错）时，按相反顺序撤销第 1 阶段的全部移动，返回 `ok=false`，`message` 写明原因和撤销结果（撤销失败会逐个列出路径）。state DB 不存在视为无需同步。
- 第 3 阶段的清理失败（包括 global state）进入 `errors`，`ok=false`，`message` 带上原因；此时数据库已提交，文件已在新位置。
- 明文和压缩文件按同一个逻辑目标判定冲突。常规移动保留原格式和字节，数据库写实际路径；修改 ID 时仍保留源格式。覆盖只替换一个明确的物理目标，回滚恢复目标原来的后缀；明文与压缩版同时存在时不移动或覆盖该组文件。
- 如果回滚时外部程序已重建源路径（任一后缀），保留新出现的文件、已移动的目标和覆盖备份，并在错误中列出位置。provider 同步仍更新 state DB 的归属，但不把显式的 `.jsonl.zst` 当作明文扫描或改写。
- 不再有 `let _ =` 丢弃 DB 或 global state 错误。

### 写 `threads` 表（`state_db::write_state_threads`）

表缺失、缺 `id`/`rollout_path`、或存在我们无法填充的 `NOT NULL` 且无默认值的列时，返回错误并列出列名，不再返回 `Ok(0)`。数据库文件不存在仍返回 `Ok(0)`。调用方：

- 旧版迁移（`legacy_migration`）：在备份之前用同一判断预检（`state_threads_write_columns`），不写 completed 标记，也不在每次重试时产生新的全库备份。错误由调用方记为 `codex_desktop_data_migration_error`。
- 导入：见下。

### 导出（`transfer::collect_export_bundle`）

在 I/O 锁内读取目录和文件；同一份快照用于 zip 条目、manifest 的 `sha256` 和 `size_bytes`。压缩会话解码成原有的 `.jsonl` 包条目，不改源文件或包版本。锁在写 zip 之前释放。

ZIP V1 不支持 Zip64。先用明文大小和压缩文件首帧声明的解码大小下界排除确定超限的选择；其余压缩文件先以流式计数核对实际大小（包含后续帧），再按实际清单、条目头和目录大小验证布局，通过后才缓冲内容。超限提示分批导出，不创建或覆盖目标包。预检后大小有变化的会话会报告单项错误。

### 导入（`transfer::apply_import_candidates`）

- 分类和确认后的复核都在 I/O 锁内读会话文件，不在用户确认期间持锁。新文件用 `OpenOptions::create_new` 写入，期间出现相同逻辑路径的文件（明文或压缩）记为冲突且不覆盖；“内容相同跳过”的文件按解码后的内容重新计算 SHA-256，变化了记为冲突。
- 单个文件失败（建目录、创建、写入）收集进 `errors` 继续处理其余文件；写入中途失败会删掉本次创建的半截文件。
- 最后对“本次写入成功 + 复核通过的相同文件”统一 upsert。upsert 失败时文件保留，`ok=false`，`message` 写明原因；重新导入同一个包会把这些文件识别为“相同跳过”并再次 upsert，从而补写索引。
- 备份改用 `VACUUM INTO`（`state_db::backup_state_database_file`），位置从 Codex home 下的 `state_5.sqlite.bak.context-manager-*` 改为数据目录 `session-manager/backups/import/`。

### 预览（`preview.rs`）

state DB 有该路径的行时不再解析会话文件；只有没有行时才走文件回退，回退出错才会让预览失败。`session_index.jsonl` 的读取警告随预览结果返回。

未压缩文件仍从末尾倒序分页；Zstandard 文件按行流式解码，只保留本页需要的消息，不写入未压缩副本。分页游标使用解码后的行偏移，后续分页校验压缩文件字节数。

### `session_index.jsonl`（`codex_home::read_session_index`）

按字节读行。非 UTF-8 的行逐行记录（行号 + 解码错误）并跳过，后面的行照常读取；读文件本身出错时记录行号并停止。两种情况都写入返回的 `warnings`，并记一条 `session_manager_session_index_read_error` 日志（路径、已读行数、条目数、跳过的行）。

### 永久删除（`delete.rs` / `codex_app_server.rs`）

- 普通和归档会话用列表返回的 thread ID，在会话 I/O 锁内通过同一 Codex Desktop app-server 调用官方 `thread/delete`。rollout、SQLite 和 `session_index.jsonl` 由官方自行清理，不直接删文件绕过它的 fork 历史依赖与子代理处理。没有回收站或撤销；升级不改旧版本留下的 `session-manager/deleted-sessions/`。
- Codex 先返回 RPC 结果，再发 `thread/deleted`；关联子代理先通知，所选根会话最后通知。同批选中已级联删除的会话时不重复发请求。删除数只统计收到通知的已选会话；收到通知的全部 ID（含子代理）一起从 `.codex-global-state.json` 清理。仅收到 RPC 成功结果但未收到删除通知，不计为成功。
- 单项错误保留 RPC 原因并继续批量操作；应用状态清理失败也返回 `ok=false` 及原始错误，不把已删除内容说成仍可恢复。旧版 Codex 不识别命令时原样报告，不退回本地文件删除。

### state DB 备份

一批归档 / 取消归档在实际有行改动时，事务前通过 `state_db::backup_state_database_with_reason` 做一次 `VACUUM INTO`。永久删除的数据库写入由官方 app-server 处理，Codex Switch 不另做数据库快照；`.codex-global-state.json` 的原有元数据备份仍保留。

### 预览的单行查找（`catalog::current_state_conversation_for_path`）

不再为一次预览构建整张目录：按目录相同的 SQL 与顺序遍历，只对文件名相同（Windows 下 ASCII 忽略大小写，与路径 key 一致）的行做 `metadata()` / `canonicalize()`，第一条解析到同一文件的行即结果——与目录里“同一文件后出现的行按重复丢弃”的规则一致。

### 会话同步读 rollout（`codex_sessions`）

- `state_threads`：`has_user_event` 已经是 1 的行不再扫描用户消息；其余情况按行流式读取，读到第一个 `session_meta` 的 cwd（通常第 1 行）并且（需要时）找到第一条含 `"user_message"` / `"user_input"` 的行就停止。子串与 `session_meta` 的判定和原来整读的实现相同。停止位置之后的内容不再解码，因此其中的非 UTF-8 字节不再让整个同步失败。
- 因其他进程占用（共享冲突 32、锁冲突 33、拒绝访问）而跳过的 rollout 仍然跳过，但数量和路径写入 `session_sync_state_db_summary` / `session_sync_preflight_state_db_summary` 的 `lockedRollouts`、`lockedRolloutPaths`。同步那条（`session_sync_state_db_summary`）从 2026-09-23 起写入时间线 `logs/codex-switch-events.jsonl`；预检查那条随定时器触发，不记录也不显示（与以前相同）。
- `rollouts::update_rollout_provider_line`：不含 `"model_provider"` 也不含 `"session_meta"` 字面量的行直接跳过 JSON 解析。
- “最近 50 个 rollout”的选择没有改（见下方决策）。

### `.codex-global-state.json` 的改写（`codex_sessions::rewrite_global_state_file`）

唯一的改写实现，会话同步（规范化工作区路径）和会话管理（删除被删/被覆盖会话的 id）都经过它：读取并解析 → 调用方修改 → 有改动时先把原文写到调用方指定的备份位置 → 仅在文件仍存在时原地改写；读写之间文件被删除则报错，不重新创建。备份位置保持原样：同步写同目录的 `.codex-global-state.json.bak`（每次覆盖），会话管理写数据目录 `session-manager/backups/<reason>/`（每次新文件，且无改动时不再创建该目录）。2026-09-23 起改写用 `atomic_file::write_file_atomically` 原子替换（写入前确认文件仍存在），写到一半崩溃不再留下截断的 global state；“确认存在”与替换之间仍有极短窗口，文件恰在此时被删会被重新写出。

### 错误日志

下列事件以 `_error` 结尾，正式版会写入数据目录 `logs/codex-switch-errors.jsonl` 和时间线 `logs/codex-switch-events.jsonl`，开发日志面板原样显示明细：

| 事件 | 主要字段 |
| --- | --- |
| `session_manager_status_state_db_error` | `root`、`targetStatus`、`conflictStrategy`、`movedFiles`、`overwrittenIds`、`error`、`rolledBack`、`rollbackErrors` |
| `session_manager_status_cleanup_error` | `root`、`targetStatus`、`changed`、`overwrittenIds`、`errors` |
| `session_manager_delete_error` | `root`、`errors` / `error` |
| `session_manager_delete_cleanup_error` | `root`、`deleted`、`globalStateError` |
| `session_manager_import_apply_error` | `root`、`candidates`、`imported`、`skipped`、`lateConflicts`、`indexedFiles`、`errors`、`sqliteError` |
| `session_manager_session_index_read_error` | `path`、`linesRead`、`entries`、`skippedLines`、`readError` |

## 2026-10-17 审查修复

- **同批重名不会互相覆盖**（提交 `dba96dc`）：两个文件名相同的会话（例如同一 uuid 分别在 `sessions/2026/05/13/` 和 `05/14/`）同批归档时，以前选“覆盖”会把第一个先移入的会话改名为覆盖备份、提交后删除，两行 state DB 指向同一个文件。现在只有计划阶段判定为覆盖（`StatusMove.overwrite`）的目标才会被替换；同批里已被前一项占用的目标按冲突处理：询问时列出、跳过时跳过、覆盖时报错并保留会话原位、修改 ID 时另取空闲名字。测试：`status_overwrite_never_replaces_a_session_moved_earlier_in_the_batch`（旧实现下失败）。
- **当时修复含非 UTF-8 行的会话列出与恢复**（提交 `2cb6057`）：前 240 行里只要有一行不是 UTF-8（例如最后一行在多字节字符中间被截断），摘要解析就整体失败，回收站里的这种会话每次恢复都会回滚，删除无法撤销；以“修改 ID”方式恢复也会因整文件转字符串失败。现在这样的行和非 JSON 行一样跳过，修改 ID 按字节处理，解析不了的行原样保留。测试：`sessions_with_a_non_utf8_line_list_and_restore_unchanged`（旧实现下失败）。
- **10 个命令统一用 `run_blocking`**（提交 `3ce279a`）；**删除记录 id 复用 `unique_backup_id`**（提交 `df1577f`）；**迁移标记改用原子写**（提交 `80d2317`，测试 `legacy_migration_writes_its_marker_and_runs_once`）。
- **zip 的 CRC32 改用 `crc32fast`**：导出、导入的每个条目都要算 CRC，原来逐位循环（每字节 8 轮移位异或）是这两步里最慢的一环；`crc32fast` 1.5.2 本来就经 png、flate2 编进了程序，现在作为直接依赖使用。测试：`crc32_matches_the_standard_check_value`（CRC-32 标准校验值），导出后导入的往返测试照常通过。提速幅度没有实测。

- **单个会话的失败写进错误日志**：某个会话失败（例如文件被占用）以前只出现在返回给界面的 `report.errors`。现在删除、归档/取消归档、导出的命令入口发现单项错误时，分别记 `session_manager_delete_error`、`session_manager_status_error`、`session_manager_export_error`（`root`、`errors` 原文）。导入原来就记 `session_manager_import_apply_error`，不重复。测试：`per_session_failures_in_a_report_are_logged`。

- **旧版数据迁移漏掉的会话写进错误日志**：迁移时个别 rollout 打不开或解析失败，以前只写进 `%APPDATA%/codex-switch/migrations/` 下的标记文件，调用方记的 `codex_desktop_data_migration` 不在任何名单里，不落盘也不显示。迁移仍按完成处理（否则每次重试都要多做一次全库备份），同时记 `codex_desktop_data_migration_rollout_error`（`root`、`rolloutFiles`、`errors`）。验证：迁移相关测试通过；这条日志没有真实触发。

- **预览读文件前拿会话 I/O 锁**：会话同步原地改写 rollout（先截断再写），预览在这期间读到的是被截断的文件，表现为“会话文件已变化”、分段读取失败或首页少了消息。现在预览与其它会话读写一样先拿 `lock_codex_session_io`，同步进行时等它结束。预览调用链里没有其它地方拿这把锁，不会重入。验证：会话管理 51 个测试通过；等待时长没有实测。

## 关键决策和原因

- **归档的顺序是“可逆的文件移动 → DB 事务 → 不可逆清理”，而不是用 DB 事务包住文件 I/O。** 持有 SQLite 写事务做文件复制（“修改 ID”要整文件重写）会让同时运行的 Codex 长时间拿不到写锁；而把删除原文件、删除覆盖备份推迟到提交之后，DB 失败时每一步都能撤回，得到同样的一致性。
- **“修改 ID”必须同时改子表。** 依据：`delete_thread_rows` 与 `write_state_threads` 按引用 thread id 处理这些子表；测试夹具里 `thread_dynamic_tools` 声明了 `FOREIGN KEY(thread_id) REFERENCES threads(id)`；项目使用的 `libsqlite3-sys 0.38.2`（bundled）编译参数含 `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`，外键默认生效。测试 `status_modify_id_moves_child_rows_with_the_renamed_thread` 先证实只改 `threads.id` 会报 `FOREIGN KEY constraint failed`——也就是旧代码在有子行时这一步必然失败，而错误此前被放进 `desktop_error` 后 `ok=true`。现在事务内先 `PRAGMA defer_foreign_keys = ON`，改完主表和子表后在提交时检查。
- **迁移在备份前预检 schema。** 迁移在启动时和每次打开 Codex 时都会重试；如果只把写入错误往上抛，不支持的 schema 会让每次重试都做一次全库 `VACUUM INTO` 备份。
- **导入的 DB 失败不删除已写入的文件。** 文件本身完整且经过 SHA-256 校验；保留它们让“重新导入”成为补写索引的途径，删除则会让用户丢掉已确认导入的内容。
- **`session_index.jsonl` 选择“逐行记录并跳过”而不是整体报错。** 它只提供标题和更新时间；一行坏数据不该让所有会话失去标题，但必须留下行号和日志。
- **不按 mtime 预筛“最近 50 个 rollout”。** 活动时间取自文件尾部事件的时间戳，mtime 与它可以无关：现有测试 `sync_uses_combined_activity_time_limit` 就把 mtime 设成与活动时间相反的顺序；导入、“修改 ID”或复制目录会让大量旧会话获得新 mtime，按 mtime 取前 2×limit 会把真正最近的会话挤出候选。同步回写会恢复 mtime（`rollouts.rs` 写后 `set_modified`），但这不足以保证等价。
- **`"model_provider"` / `"session_meta"` 字面量预筛的前提**：Codex 用 serde 写 JSONL，键名不会被转义（`_` 之类）。只有这种人为转义的行才会与“逐行解析”的结果不同。
- **`.codex-global-state.json` 的写入放在 `codex_sessions`。** `session_manager` 已依赖 `codex_sessions`（I/O 锁），反向不行。两套旧实现都不是原子写（一个 `fs::write`，一个截断后原地写）；合并时保留了“只写已存在的文件”的一套，之后在 `main` 上换成了 `atomic_file::write_file_atomically`。
- **rollout 回写仍是原地写，但写前复核（2026-09-23）。** 关闭 Codex 失败后的直接同步、重开编辑器前的同步都可能在 Codex 仍运行时改写 rollout。这里不能换成临时文件 + rename：Codex 手里的追加句柄会继续写进被替换掉的旧文件，新内容永久丢失。改为写入前比对文件长度和修改时间，与读取时不一致（Codex 在这期间追加过）就跳过该文件并作为同步错误报告，避免截断掉新追加的行；复核与写入之间仍有极短窗口。测试 `rollout_appended_after_the_read_is_not_rewritten`。
- **单元测试不写真实数据目录。** `backup::session_manager_data_dir` 在 `cfg(test)` 下指向系统临时目录 `codex-switch-session-manager-tests`（与 `app_log` 的日志文件同一做法）。此前已有测试会把 state DB 备份写进真实 `%APPDATA%\codex-switch\session-manager\backups`。

## 验证记录

### 2026-11-18：会话扫描提示

- 离线前端行为测试 `node --experimental-vm-modules --test scripts/test-session-delete.mjs`：6 项通过。模拟目录部分成功，核对可用行、失败计数、重复提示与原始错误；模拟整次刷新失败、首次失败、后续成功清除提示及并发扫描结果乱序，保留原有两项删除回归。未启动安装版。

### 2026-11-18：目录与压缩预览

- Windows 隔离运行，Codex app-server `0.169.0-alpha.11.1`：`app_server_lists_and_previews_compressed_rollouts_across_pages` 先对 160 条未归档 / 20 条已归档会话建立索引，再把 130 个 rollout 替换成 `.jsonl.zst`。真实 app-server 跨页重查仍返回全部 180 条，归档计数仍为 20；压缩归档会话的预览可读。未操作正式库或前台界面。
- 离线用例覆盖逻辑路径去重、缺失文件报错、与未压缩历史一致的消息分页或响应项解析、旧快照和无效 Zstandard 流的错误。
- 同版官方 CLI 在隔离夹具中归档和取消归档后，数据库写实际 `.jsonl.zst`，RPC 仍返回逻辑 `.jsonl`。会话管理离线用例覆盖压缩归档往返字节不变、修改 ID 后仍能解码、跨格式覆盖遇 DB 故障的双文件回滚、源路径已重建时不覆盖、V1 导入/导出往返与迟到的压缩文件冲突；状态同步用例确认不会把压缩流里的字节标记误认成用户消息。
- ZIP 定向用例覆盖多帧压缩实际大小、大小变化、清单把包推过上限、条目数和目录开销，以及布局不合法时不截断现有目标文件。

### 2026-11-17：永久删除

- Windows 隔离运行，安装版 Codex app-server `0.169.0-alpha.11.1`。`permanent_delete_uses_official_app_server_in_isolated_home` 由合成 rollout 建立七条会话；批量删除选中的未归档会话、归档会话和子代理，另一个未选中的子代理由 Codex 级联删除，不计入选中数量。仍被 fork 历史引用的会话返回 `-32600`，该会话、fork 和无关会话均保留。
- 回读 rollout 文件、`session_index.jsonl`、`state_5.sqlite`、官方会话列表和 `.codex-global-state.json`：已删项消失，保留项不变。运行使用独立 HOME、APPDATA、LOCALAPPDATA 和临时目录，不读写真实会话；未验证 macOS 上的真实操作。
- 离线：`permanent_delete_reports_rpc_failures_and_cleans_only_successes` 验证部分失败的报告及仅移除成功项的应用状态。真实 app-server 测试默认忽略，在隔离环境显式运行 `cargo test permanent_delete_uses_official_app_server_in_isolated_home -- --ignored`。
- 完整检查：`npm run check`（72 passed / 4 skipped）、`cargo test --locked`（372 passed / 7 ignored）、fmt 和 Clippy all-targets 通过。前端定向 VM 回归验证确认前不删除、单次 IPC、批量部分失败提示及删除后清除旧预览；未操作安装版 UI。

### 2026-09-23：一致性与错误可见性修复（离线）

- 新增/改写的单元测试（临时目录 + 真实 SQLite 文件）：
  - `status_db_failure_moves_files_back_and_reports_reason`：`threads` 缺 `archived_at` → 文件移回原位、`ok=false`、`message` 含列名和“已撤销 1 个文件移动”。
  - `status_modify_id_moves_child_rows_with_the_renamed_thread`：外键前提（见上）+ 修改 ID 后 `threads`、`thread_dynamic_tools`、`thread_spawn_edges` 都指向新 id，`PRAGMA foreign_key_check` 为空，原归档文件未变。
  - `status_overwrite_removes_overwritten_rows_and_surfaces_global_state_failure`：被覆盖会话的行和子行在同一事务删除；global state 损坏时 `ok=false` 且 `message` 带原因；覆盖备份已清理。
  - `upsert_state_threads_rejects_schemas_it_cannot_write`（替换原 `..._skips_unsupported_required_schema`）：三种不支持的 schema 都返回带列名的错误且不写入。
  - `restore_keeps_trash_when_state_schema_cannot_be_written`、`legacy_migration_rejects_unwritable_schema_without_marker_or_backup`。
  - `export_bundle_manifest_matches_exported_bytes_and_imports_cleanly`：manifest 的 sha/size 与 zip 字节一致，并能通过导入端的严格校验。只验证了不变式；“读取期间文件被追加”的竞态没有在测试里复现。
  - `import_apply_never_overwrites_late_files_and_indexes_what_was_written`：迟到的同名文件不被覆盖、坏路径不影响其余文件、只索引实际落盘的文件。
  - `state_db_backup_includes_uncheckpointed_wal_pages`：先证实 WAL 模式下 `fs::copy` 主文件拿不到未 checkpoint 的行，再证实 `VACUUM INTO` 备份包含全部行。
  - `preview_falls_back_to_rollout_when_state_db_has_no_row`、`session_index_skips_non_utf8_lines_and_keeps_reading`。预览“state DB 命中时不再解析文件”只做了代码审查，没有可观测的测试点。
- 本地检查：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 通过；`cargo test` 277 passed / 5 ignored（修改前 267 / 5；session_manager 用例 28 → 38）。测试进程的 `USERPROFILE`/`HOME`/`APPDATA` 指向临时目录，没有运行 `#[ignore]` 的真实环境用例。
- 未验证：没有运行正式应用，没有接触真实 `~/.codex` 或数据目录，上述行为都没有在真实 Codex 数据上跑过。

### 2026-09-23：读写开销与重复实现的清理（离线）

- 对比测试（新实现与旧逻辑在同一批夹具上结果相同）：`streamed_rollout_metadata_matches_full_read`（9 种 rollout 形态 + 缺失文件）、`provider_line_prefilter_matches_full_parse`（16 行 × 2 个目标 provider）、`single_path_lookup_matches_the_full_catalog`（重复写法、相对路径、同名异目录、缺失、目录外、Windows 大小写变体）。
- 行为测试：`state_sync_reads_only_what_the_row_still_needs`（已记录 user event 的行只读第 1 行，后面的非 UTF-8 内容不影响同步）、`state_sync_reports_rollouts_locked_by_another_process`（Windows，`share_mode(0)` 真实占用文件，记入 `lockedRollouts` 且同步成功）、`restore_rejects_a_same_size_corrupted_backup`、`restore_legacy_record_without_hash_still_restores`、`state_db_backup_is_taken_once_per_batch_and_only_when_rows_change`、`status_change_without_state_rows_takes_no_backup`、`global_state_rewrite_backs_up_only_changes_and_never_recreates`、`global_state_cleanup_writes_through_the_shared_rewrite_with_a_data_dir_backup`。
- 本地检查（HEAD `3a7eb2f`）：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 通过；`cargo test` 288 passed / 5 ignored。测试进程的 `USERPROFILE`/`HOME`/`APPDATA` 指向临时目录。
- 未验证：没有在真实数据规模上测量耗时（上千个 rollout、上百 MB）；性能收益只来自读取量和系统调用次数的推算。

### 2026-09-23：逐个提交的完整检查与备份文件名竞争

- 对 `347da53..83c7bf9` 的每个提交分别检出并运行 fmt、Clippy all-targets 和完整 `cargo test`：除 `71bb037` 外全部通过，测试数从 267 逐步增加到 288（均 5 ignored）。`71bb037` 那一次是 269 passed / 1 failed，耗时 7.26s（其余约 3.5s）；失败输出没有保存，之后在该提交上重跑 17 次都通过，没能确认是哪个用例。
- 排查中证实了一个真实的竞争：备份文件名只精确到秒，并用“先检查是否存在再创建”选名。8 个线程并发备份（同一 reason）时 160 次里 133 次失败，报 `table threads already exists`。正式运行中所有备份都在会话 I/O 锁内顺序执行，不受影响；但单元测试直接并行调用不加锁的内部函数。修复为先用 `create_new` 占住文件名再写入（`backup::reserve_reason_backup_file`），并加了并发测试 `concurrent_backups_with_the_same_reason_get_distinct_files`。
- 修复后（`664fafc`）完整 `cargo test` 289 passed / 5 ignored，连续 6 次均通过。`71bb037` 那次失败是否就是这个竞争，仍是推断。

### 2026-11-27：真实 `state_5.sqlite` 结构核对、隔离真实运行与 `347da53` 对比基准

- **真实 `state_5.sqlite` 外键结构与“修改 ID”归档验证：** 只读检查本机正式 `state_5.sqlite`（64 条 SQLx 迁移、5892 条 `threads`），确认以下子表声明了 `REFERENCES threads(id) ON DELETE CASCADE`（默认 `ON UPDATE NO ACTION`、非 `DEFERRABLE`）：`thread_dynamic_tools`、`thread_attachments`、`guardian_review_feedback`；另有无外键的 `thread_read_receipts`、`thread_spawn_edges` 与 `threads.attached_thread_parent_id`。据此将 `state_db.rs` 简化为单一表清单并补齐上述 4 处真实关联。在隔离沙盒中基于真实 `state_5.sqlite` 备份对含上述关联行的会话执行 `modify_id` 归档，返回 `ok=true`、`desktop_error=null`，`PRAGMA foreign_key_check` 为 0，所有子表中均无旧 id 残留。
- **归档 DB 失败回滚（隔离真实运行）：** 对沙盒 `state_5.sqlite` 持有真实 SQLite `BEGIN IMMEDIATE` 写锁超过 `busy_timeout` 后触发归档，返回 `database is locked` 并提示已撤销 1 个文件移动；沙盒 `logs/codex-switch-errors.jsonl` 记下 `session_manager_status_state_db_error`（`movedFiles: 1`、`rolledBack: 1`、`rollbackErrors: []`），原会话文件完好回到原目录。
- **真实规模流式读取与单行查找基准（对比 `347da53`）：** 在两份完全相同的隔离真实数据副本（完整 5899 行 `state_5.sqlite` + 65 份真实 rollout 共 237.76 MB，全量 `api -> openai` 且清空 `cwd`/`has_user_event` 强制读文件）上对比：
  - 首次对比暴露真实性能退化：`state_threads::contains_bytes` 使用逐字节 `.windows(needle.len())` 扫描，导致同步耗时 `7105 ms`，慢于 `347da53` 的 `2271 ms`。改为复用已有依赖 `memchr::memmem::find` 后，温态同步降至 **`2295 ms`**（`347da53` 为 `2386 ms`），`session_sync_state_db_summary` 报告 `updated: 5964`（`providerRowsUpdated: 5899`、`cwdRowsUpdated: 65`、`lockedRollouts: 0`），两版本同步后的数据库行完全一致。
  - `catalog::current_state_conversation_for_path` 连续 20 次查找在 5899 行库上从 `347da53` 的 **`115.15 ms/次`（共 `2303.06 ms`）** 降至 **`2.02 ms/次`（共 `40.42 ms`）**，提速约 57 倍。

## 待验证

（当前模块已登记的 `TODO(verify)` 项均已完成验证并归入上方验证记录。）

## 待决事项

- state DB / global state 备份的保留与清理策略（目前每次有改动的批量操作都新增一份 `VACUUM INTO` 备份，永不清理）。
- `.codex-global-state.json` 的两个备份位置是否统一（同步：同目录 `.bak` 覆盖式；会话管理：数据目录按次保留）。

## 回退

revert 对应提交后重新构建。每次写 state DB 前的 `VACUUM INTO` 备份在数据目录 `session-manager/backups/<reason>/`，可用于恢复数据库，不会恢复已永久删除的会话内容。
