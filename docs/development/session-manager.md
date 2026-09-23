# 会话管理：一致性与错误可见性

范围：`src-tauri/src/session_manager/`（归档、删除/恢复、导入导出、预览、旧版数据迁移）。读写会话数据仍须先拿 `codex_sessions::lock_codex_session_io(...)`（见 CONTRIBUTING.md）。

## 当前行为

### 归档 / 取消归档（`status.rs`）

同一把 I/O 锁内分三个阶段：

1. 文件移动，全部可逆：普通移动用 `rename`；“修改 ID”先写一份改了 ID 的副本，原文件保留；“覆盖”先把目标改名为备份。计划阶段认为空闲、执行时却已存在的目标不会被替换，记为失败。
2. 一个 state DB 事务（`state_db::apply_status_moves_to_state_db`）：先删除被覆盖会话的行及其子表行，再更新每个移动行的 `archived`、`archived_at`、`rollout_path`；“修改 ID”时同一事务内把子表（`thread_dynamic_tools`、`thread_goals`、`thread_spawn_edges`、`stage1_outputs`、`agent_job_items`）引用的 thread id 一起改掉。整批只备份一次数据库。
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

## 待验证

TODO(verify): 真实 Codex 的 `state_5.sqlite` 中 thread 子表是否声明了外键、声明方式（`ON UPDATE` 动作、是否 `DEFERRABLE`）还没有看过；本次按测试夹具和现有删除逻辑推断。触发条件：下一次在正式版里用“修改 ID”处理归档/取消归档冲突，且该会话有动态工具或子代理关系。检查：返回 `ok=true`；数据目录 `logs/codex-switch-errors.jsonl` 没有 `session_manager_status_state_db_error`；只读查询 `SELECT thread_id FROM thread_dynamic_tools` / `thread_spawn_edges` 中不再出现旧 id。判据不成立（日志 `error` 含 `FOREIGN KEY` 或其他 SQLite 错误）时，从 `state_db::rename_thread_references` 与该日志的 `rollbackErrors` 继续；用户可用 `sqlite3 state_5.sqlite ".schema thread_dynamic_tools"` 只读提供真实建表语句。

TODO(verify): 归档 DB 失败后的文件撤销只在临时目录验证过。触发条件：正式版中出现 `session_manager_status_state_db_error`。检查该事件的 `rolledBack` 等于 `movedFiles`、`rollbackErrors` 为空，并确认对应会话文件回到原目录。判据不成立时按 `rollbackErrors` 中的路径手工处理，并从 `status::rollback_status_move_file` 继续。

## 回退

revert 对应提交后重新构建。每次写 state DB 前的 `VACUUM INTO` 备份在数据目录 `session-manager/backups/<reason>/`，可用于恢复数据库。
