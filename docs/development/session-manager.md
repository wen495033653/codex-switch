# 会话管理与会话同步的文件读写

范围：`src-tauri/src/session_manager/`（归档、删除/恢复、导入导出、预览、旧版数据迁移），以及 `src-tauri/src/codex_sessions/` 中读写 rollout、`state_5.sqlite`、`.codex-global-state.json` 的部分。Codex 的结束与重启流程见 [codex-restart.md](codex-restart.md)。读写会话数据仍须先拿 `codex_sessions::lock_codex_session_io(...)`（见 CONTRIBUTING.md）。

## 当前行为

### 归档 / 取消归档（`status.rs`）

同一把 I/O 锁内分三个阶段：

1. 文件移动，全部可逆：普通移动用 `rename`；“修改 ID”先写一份改了 ID 的副本，原文件保留；“覆盖”先把目标改名为备份。计划阶段认为空闲、执行时却已存在的目标不会被替换，记为失败。
2. 一个 state DB 事务（`state_db::apply_status_moves_to_state_db`）：先删除被覆盖会话的行及其子表行，再更新每个移动行的 `archived`、`archived_at`、`rollout_path`；“修改 ID”时同一事务内把子表（`thread_dynamic_tools`、`thread_goals`、`thread_spawn_edges`、`stage1_outputs`、`agent_job_items`）引用的 thread id 一起改掉。整批只备份一次数据库；这些会话在 `threads` 和子表里都没有行时，不备份也不开事务。
3. 事务成功后才做不可逆清理：删除“修改 ID”的原文件和覆盖备份、清理空目录，再清理 `.codex-global-state.json` 里被覆盖会话的 id。

- DB 失败（打不开、缺 `threads` 表或缺 `id/archived/archived_at/rollout_path` 列、SQLite 报错）时，按相反顺序撤销第 1 阶段的全部移动，返回 `ok=false`，`message` 写明原因和撤销结果（撤销失败会逐个列出路径）。state DB 不存在视为无需同步。
- 第 3 阶段的清理失败（包括 global state）进入 `errors`，`ok=false`，`message` 带上原因；此时数据库已提交，文件已在新位置。
- 不再有 `let _ =` 丢弃 DB 或 global state 错误。

### 写 `threads` 表（`state_db::write_state_threads`）

表缺失、缺 `id`/`rollout_path`、或存在我们无法填充的 `NOT NULL` 且无默认值的列时，返回错误并列出列名，不再返回 `Ok(0)`。数据库文件不存在仍返回 `Ok(0)`。调用方：

- 恢复（`trash_store`）：DB 没写入 → 删除刚恢复的文件、回滚被覆盖目标、保留回收站记录。
- 旧版迁移（`legacy_migration`）：在备份之前用同一判断预检（`state_threads_write_columns`），不写 completed 标记，也不在每次重试时产生新的全库备份。错误由调用方记为 `codex_desktop_data_migration_error`。
- 导入：见下。

### 导出（`transfer::collect_export_bundle`）

在 I/O 锁内读取目录和文件；每个文件只读一次，同一份字节用于 zip 条目、manifest 的 `sha256` 和 `size_bytes`。锁在写 zip 之前释放。

### 导入（`transfer::apply_import_candidates`）

- 确认对话框里的分类是在不持锁时做的；用户确认后在锁内复核：新文件用 `OpenOptions::create_new` 写入，期间出现的同名文件记为冲突且不覆盖；“内容相同跳过”的文件重新计算 SHA-256，变化了记为冲突。
- 单个文件失败（建目录、创建、写入）收集进 `errors` 继续处理其余文件；写入中途失败会删掉本次创建的半截文件。
- 最后对“本次写入成功 + 复核通过的相同文件”统一 upsert。upsert 失败时文件保留，`ok=false`，`message` 写明原因；重新导入同一个包会把这些文件识别为“相同跳过”并再次 upsert，从而补写索引。
- 备份改用 `VACUUM INTO`（`state_db::backup_state_database_file`），位置从 Codex home 下的 `state_5.sqlite.bak.context-manager-*` 改为数据目录 `session-manager/backups/import/`。

### 预览（`preview.rs`）

state DB 有该路径的行时不再解析会话文件；只有没有行时才走文件回退，回退出错才会让预览失败。`session_index.jsonl` 的读取警告随预览结果返回。

### `session_index.jsonl`（`codex_home::read_session_index`）

按字节读行。非 UTF-8 的行逐行记录（行号 + 解码错误）并跳过，后面的行照常读取；读文件本身出错时记录行号并停止。两种情况都写入返回的 `warnings`，并记一条 `session_manager_session_index_read_error` 日志（路径、已读行数、条目数、跳过的行）。

### 回收站的完整性校验（`trash_store.rs`）

- 列出回收站、翻页预览、规划恢复时只检查备份文件存在且大小与记录一致，不再整文件计算 SHA-256。
- 恢复时在即将放到位的副本上校验：普通恢复在写完临时文件后计算其 SHA-256，必须等于记录值；“修改 ID”恢复把备份整读一次，先用这份内存快照对比记录值，再从快照改写 ID。没有 `sha256` 的旧记录仍比较副本与备份。校验不通过时目标不动、临时文件删除、回收站记录保留。
- 删除时只对回收站副本算一次 SHA-256；删除原文件前原有的“对原文件再算一次并比较”同时证明副本完整、原文件没有变化（不一致时保留原文件，提示改为“复制期间发生变化或复制不完整”）。
- 恢复后给 `thread_metadata_from_manifest` 的 manifest 不再计算目标文件哈希（该函数只读 id、标题、更新时间、状态）。

### state DB 备份（`state_db::StateDbBatchBackup`）

一次批量操作（删除一批、恢复一批、归档一批）最多做一次 `VACUUM INTO`，在第一条真正改动行的语句之前；传入的 id 在 `threads` 和引用 thread id 的子表中都没有行时不备份。没有子行也没有主表行的 id 不再触发备份；只有子行（孤儿行）的 id 仍会被删除并触发备份。备份保留/清理策略没有改动。

### 预览的单行查找（`catalog::current_state_conversation_for_path`）

不再为一次预览构建整张目录：按目录相同的 SQL 与顺序遍历，只对文件名相同（Windows 下 ASCII 忽略大小写，与路径 key 一致）的行做 `metadata()` / `canonicalize()`，第一条解析到同一文件的行即结果——与目录里“同一文件后出现的行按重复丢弃”的规则一致。

### 会话同步读 rollout（`codex_sessions`）

- `state_threads`：`has_user_event` 已经是 1 的行不再扫描用户消息；其余情况按行流式读取，读到第一个 `session_meta` 的 cwd（通常第 1 行）并且（需要时）找到第一条含 `"user_message"` / `"user_input"` 的行就停止。子串与 `session_meta` 的判定和原来整读的实现相同。停止位置之后的内容不再解码，因此其中的非 UTF-8 字节不再让整个同步失败。
- 因其他进程占用（共享冲突 32、锁冲突 33、拒绝访问）而跳过的 rollout 仍然跳过，但数量和路径写入 `session_sync_state_db_summary` / `session_sync_preflight_state_db_summary` 的 `lockedRollouts`、`lockedRolloutPaths`（DEV 日志面板也展示）。这两个事件不是 `_error`，正式版不落盘。
- `rollouts::update_rollout_provider_line`：不含 `"model_provider"` 也不含 `"session_meta"` 字面量的行直接跳过 JSON 解析。
- “最近 50 个 rollout”的选择没有改（见下方决策）。

### `.codex-global-state.json` 的改写（`codex_sessions::rewrite_global_state_file`）

唯一的改写实现，会话同步（规范化工作区路径）和会话管理（删除被删/被覆盖会话的 id）都经过它：读取并解析 → 调用方修改 → 有改动时先把原文写到调用方指定的备份位置 → 仅在文件仍存在时原地改写；读写之间文件被删除则报错，不重新创建。备份位置保持原样：同步写同目录的 `.codex-global-state.json.bak`（每次覆盖），会话管理写数据目录 `session-manager/backups/<reason>/`（每次新文件，且无改动时不再创建该目录）。2026-09-23 起改写用 `atomic_file::write_file_atomically` 原子替换（写入前确认文件仍存在），写到一半崩溃不再留下截断的 global state；“确认存在”与替换之间仍有极短窗口，文件恰在此时被删会被重新写出。

### 错误日志

下列事件以 `_error` 结尾，正式版会写入数据目录 `logs/codex-switch-errors.jsonl`（DEV 日志面板不展示这些新事件的明细）：

| 事件 | 主要字段 |
| --- | --- |
| `session_manager_status_state_db_error` | `root`、`targetStatus`、`conflictStrategy`、`movedFiles`、`overwrittenIds`、`error`、`rolledBack`、`rollbackErrors` |
| `session_manager_status_cleanup_error` | `root`、`targetStatus`、`changed`、`overwrittenIds`、`errors` |
| `session_manager_restore_state_db_error` | `deleteId`、`root`、`target`、`trashRetained`、`error` |
| `session_manager_import_apply_error` | `root`、`candidates`、`imported`、`skipped`、`lateConflicts`、`indexedFiles`、`errors`、`sqliteError` |
| `session_manager_session_index_read_error` | `path`、`linesRead`、`entries`、`skippedLines`、`readError` |

## 关键决策和原因

- **归档的顺序是“可逆的文件移动 → DB 事务 → 不可逆清理”，而不是用 DB 事务包住文件 I/O。** 持有 SQLite 写事务做文件复制（“修改 ID”要整文件重写）会让同时运行的 Codex 长时间拿不到写锁；而把删除原文件、删除覆盖备份推迟到提交之后，DB 失败时每一步都能撤回，得到同样的一致性。
- **“修改 ID”必须同时改子表。** 依据：`delete_state_threads_for_sessions` 与 `write_state_threads` 已把这些子表当作引用 thread id 的表；测试夹具里 `thread_dynamic_tools` 声明了 `FOREIGN KEY(thread_id) REFERENCES threads(id)`；项目使用的 `libsqlite3-sys 0.38.2`（bundled）编译参数含 `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`，外键默认生效。测试 `status_modify_id_moves_child_rows_with_the_renamed_thread` 先证实只改 `threads.id` 会报 `FOREIGN KEY constraint failed`——也就是旧代码在有子行时这一步必然失败，而错误此前被放进 `desktop_error` 后 `ok=true`。现在事务内先 `PRAGMA defer_foreign_keys = ON`，改完主表和子表后在提交时检查。
- **迁移在备份前预检 schema。** 迁移在启动时和每次打开 Codex 时都会重试；如果只把写入错误往上抛，不支持的 schema 会让每次重试都做一次全库 `VACUUM INTO` 备份。
- **导入的 DB 失败不删除已写入的文件。** 文件本身完整且经过 SHA-256 校验；保留它们让“重新导入”成为补写索引的途径，删除则会让用户丢掉已确认导入的内容。
- **`session_index.jsonl` 选择“逐行记录并跳过”而不是整体报错。** 它只提供标题和更新时间；一行坏数据不该让所有会话失去标题，但必须留下行号和日志。
- **回收站只在恢复时校验内容。** 完整性要保证的是“恢复出来的内容等于删除时的内容”，在要放到位的副本上校验一次就够；列表和预览只是展示。代价：同样大小但内容损坏的备份仍会出现在列表里，恢复时才报 SHA-256 不匹配（测试 `restore_rejects_a_same_size_corrupted_backup` 固定了这一行为）。
- **不按 mtime 预筛“最近 50 个 rollout”。** 活动时间取自文件尾部事件的时间戳，mtime 与它可以无关：现有测试 `sync_uses_combined_activity_time_limit` 就把 mtime 设成与活动时间相反的顺序；导入、恢复、“修改 ID”或复制目录会让大量旧会话获得新 mtime，按 mtime 取前 2×limit 会把真正最近的会话挤出候选。同步回写会恢复 mtime（`rollouts.rs` 写后 `set_modified`），但这不足以保证等价。
- **`"model_provider"` / `"session_meta"` 字面量预筛的前提**：Codex 用 serde 写 JSONL，键名不会被转义（`_` 之类）。只有这种人为转义的行才会与“逐行解析”的结果不同。
- **`.codex-global-state.json` 的写入放在 `codex_sessions`。** `session_manager` 已依赖 `codex_sessions`（I/O 锁），反向不行。两套旧实现都不是原子写（一个 `fs::write`，一个截断后原地写）；合并时保留了“只写已存在的文件”的一套。本分支基于 `347da53`，那里还没有 `atomic_file`；`main` 上已有 `atomic_file::write_file_atomically`，是否换成它见待决事项。
- **单元测试不写真实数据目录。** `backup::session_manager_data_dir` 在 `cfg(test)` 下指向系统临时目录 `codex-switch-session-manager-tests`（与 `session_sync_diagnostics` 的错误日志同一做法）。此前已有测试会把 state DB 备份写进真实 `%APPDATA%\codex-switch\session-manager\backups`。

## 验证记录

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

## 待验证

TODO(verify): 真实 Codex 的 `state_5.sqlite` 中 thread 子表是否声明了外键、声明方式（`ON UPDATE` 动作、是否 `DEFERRABLE`）还没有看过；本次按测试夹具和现有删除逻辑推断。触发条件：下一次在正式版里用“修改 ID”处理归档/取消归档冲突，且该会话有动态工具或子代理关系。检查：返回 `ok=true`；数据目录 `logs/codex-switch-errors.jsonl` 没有 `session_manager_status_state_db_error`；只读查询 `SELECT thread_id FROM thread_dynamic_tools` / `thread_spawn_edges` 中不再出现旧 id。判据不成立（日志 `error` 含 `FOREIGN KEY` 或其他 SQLite 错误）时，从 `state_db::rename_thread_references` 与该日志的 `rollbackErrors` 继续；用户可用 `sqlite3 state_5.sqlite ".schema thread_dynamic_tools"` 只读提供真实建表语句。

TODO(verify): 归档 DB 失败后的文件撤销只在临时目录验证过。触发条件：正式版中出现 `session_manager_status_state_db_error`。检查该事件的 `rolledBack` 等于 `movedFiles`、`rollbackErrors` 为空，并确认对应会话文件回到原目录。判据不成立时按 `rollbackErrors` 中的路径手工处理，并从 `status::rollback_status_move_file` 继续。

TODO(verify): 流式读取与单行查找的耗时收益没有在真实规模上测过。触发条件：下一次按 [dev-preview.md](dev-preview.md) 在隔离环境里用真实数据的副本运行开发版。检查：切换模式后 `session_sync_start` 到 `session_sync_finish` 的时间差、会话管理页连续翻页的响应时间，与 `347da53` 构建在同一份数据副本上对比；`session_sync_state_db_summary` 的 `updated` 与 `lockedRollouts` 合理。判据：同步与翻页不慢于旧版本且数据库结果一致。判据不成立时从 `state_threads::read_rollout_thread_metadata` 与 `catalog::current_state_conversation_for_path` 继续。

## 待决事项

- state DB / global state 备份的保留与清理策略（目前每次有改动的批量操作都新增一份 `VACUUM INTO` 备份，永不清理）。
- `.codex-global-state.json` 的两个备份位置是否统一（同步：同目录 `.bak` 覆盖式；会话管理：数据目录按次保留）。

## 回退

revert 对应提交后重新构建。每次写 state DB 前的 `VACUUM INTO` 备份在数据目录 `session-manager/backups/<reason>/`，可用于恢复数据库。
