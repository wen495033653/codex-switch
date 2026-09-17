# Codex 的结束、重启与会话同步

## 流程

- watcher 发现 Codex 打开后，如果开启了会话同步且会话的 provider 与当前模式不一致，就走“结束 Codex → 同步会话 → 以 CDP 模式重新打开”。界面上的“重启 Codex”（`restart_current_codex_app_normal`）走同样的“结束 → 同步 → 重新打开”，以普通模式打开。
- 命令都是 async + `spawn_blocking`，等待进程不占用调用线程。
- 结束前校验可执行路径。`taskkill` 最长 10000ms，超时就结束该命令；清理最多 1000ms，stdout 和 stderr 各最多等 500ms；之后等待目标退出的上限是 12000ms。两路输出并发读取，避免管道写满。
- 失败时返回 PID、退出状态、单独的 `exitCode`、`elapsedMs`、两路输出和清理结果，非 UTF-8 输出用 `rawBase64` 保留原字节。所有调用方都向上传递 `Result`，不忽略错误。
- 普通模式启动后，确认同一个子进程句柄持续存活 1500ms；提前退出（包括 `exitCode=0`）按失败返回，不重试、不伪装成功。这只证明进程存活，不代表窗口或业务就绪。CDP 模式用实际的 CDP 注入来确认。
- 相关日志事件：`codex_app_process_kill_error` / `_finish`、`codex_app_process_kill_tree_exited`、`codex_app_launch_confirmation_error` / `_finish`、`codex_app_restart_command_error`、`codex_app_watcher_on_open_error`。`*_error` 事件从 v5.4.12 起会写入数据目录下的 `logs/codex-switch-errors.jsonl`。

## 结束进程树：只看“是否全部退出”（2026-09-17）

### 现象与根因

- 切到 API 模式后，`state_5.sqlite` 里 1605 个 thread 仍是 `model_provider=openai`，watcher 每次看到 Codex 打开都判定同步待处理并走重启流程。
- `taskkill /F /T` 结束进程树时，部分后代（例如 app-server 下的 `cmd /c` 包装进程）会在子进程被终止后自己退出，轮到它们时 taskkill 报“没有此任务的实例在运行”，整体退出码 128。对已经随主进程退出的 helper 单独 taskkill 同样返回 128。
- 旧的 `kill_process_tree` 把 taskkill 的退出码当成成功判据，`relaunch_running_codex_processes` 里的 `?` 随即返回：Codex 已被结束，但同步和重启都被跳过，数据库仍是 `openai`，下次打开再次循环。用户看到的就是 Codex 启动约 10 秒后被关掉且不再打开。
- 现场证据：外部监控记录 codex-switch 对 ChatGPT 主进程 taskkill 退出码 0，随后对 helper PID 的 taskkill 退出码 128，之后没有重启；Codex 日志没有退出记录，也没有 Crashpad dump；直接调用修复前的 `handle_codex_app_open` 得到 `exitCode=Some(128)`，输出为 28 条“成功”和 2 条“没有此任务的实例在运行”。

### 实现（唯一实现，不保留按退出码判定的旧逻辑）

- `kill_process_tree`：先快照目标进程树（根和全部后代），执行终止（Windows 用 `taskkill /F /T`，其他平台逐个 kill），再等待快照内的进程退出，上限 5000ms。成功只由“进程树全部退出”判定，两个平台一致。终止命令的返回值只作为依据记录：失败但树已退出时记 `codex_app_process_kill_tree_exited`（含 `treePids` 和 `terminationError` 原文）；仍有存活则返回错误，附存活 PID、`treePids` 和终止结果。
- `kill_root_process_trees`：重启 Codex 和重开编辑器时只结束根进程（父进程不在同一应用的进程集合里），后代随进程树结束，之后仍等待全部 PID 退出。实测逐个结束 Electron helper 会让主进程把 helper 重新拉起，9 次 taskkill 约 9 秒后才轮到主进程。
- watcher 识别根进程和上面的调用共用 `root_pids`。

## 验证记录

### 2026-09-06：重启调度

- `cargo test process_control::tests` 8 项通过：真实独立子进程的超时与清理、退出码与两路输出、大量输出不阻塞、提前退出 0 和 23、存活确认后实际 taskkill、错误返回与日志字段。
- 跨平台退出码：CI run 34031186378 的 macOS 上，退出码 23 的子进程被记录成 `ExitStatus(unix_wait_status(5888))`，只输出平台相关的 Debug 文本导致断言失败。现在同时保留原始 status 和 `ExitStatus::code()` 得到的 `exitCode=Some(23)`；超时且没有确认退出码时记 `exitCode=None`。
- 没有结束或重启真实的 Codex。

### 2026-09-17：结束进程树

- 离线：`cargo fmt --check`、`cargo test`（243 passed、3 ignored）、`cargo clippy` 通过。其中 `taskkill_not_found_for_already_exited_tree_is_success_and_logged` 用真实 taskkill 触发退出码 128 并检查日志；`process_tree_kill_outcome_is_decided_by_tree_exit_only` 覆盖“终止成功但仍存活”“终止失败但已退出”等判定；`root_tree_kill_ends_descendants_without_killing_them_separately` 用真实父子进程确认只结束根进程就能清掉子进程。
- CI run 35127395827（commit `bd788c5`）：renderer、Windows、macOS 全部通过。
- 真实运行，最终实现（01:20，暂停正式版，测试二进制调用 `restart_current_codex_app_normal` 作用于运行中的 Codex）：外部监控只看到一次针对 ChatGPT 根进程的 `taskkill /F /T`，退出码 128；日志 `codex_app_process_kill_tree_exited` → `codex_app_process_kill_finish terminated=true`；01:20:50 全部 ChatGPT.exe 退出，01:20:52 以普通模式重新打开（`restartedCount=1`），之后持续运行。
- 真实运行，中间版本（01:01，测试二进制调用 `handle_codex_app_open`）：taskkill 退出码 128 后流程继续，会话同步完成（`threads.model_provider` 全部变为 `api`，1596 个加归档 10 个），01:02:08 以 `--remote-debugging-port=9229` 启动 Codex。该测试进程没有安装 rustls crypto provider（正式程序在 `main.rs` 启动时安装），CDP 注入请求在 reqwest 内 panic，所以 CDP 注入和 `relaunch_running_codex_processes` 的完成结果没有被验证。

### 2026-09-17 晚：v6.0.0 代码，“重启 Codex”路径

- 背景：18:20 从 API 模式切回订阅模式后，同步一直没有执行（当时安装的 5.4.11 没有上面的修复，用户随后退出了它）。23:19 时 `threads.model_provider` 为 `api` 1619 个（含归档 11 个）、`openai` 5 个，Codex 打开旧会话报 “Model provider `api` not found”。
- 真实运行：当时唯一在运行的 codex-switch 是 v6.0.0 代码的 Dev 预览构建（预览模式，不启动 watcher）。23:33:52 出现新的 ChatGPT 主进程，命令行没有 `--remote-debugging-port`（普通模式），到 23:51 仍在运行；23:41 查询 `threads.model_provider` 全部为 `openai`（1625 个）；没有生成错误日志文件。推断是用户在界面上点了“重启 Codex”，预览构建的内存日志没有保留，无法从日志确认。
- 没有验证：watcher 路径（自动接管并以 CDP 模式重启）这次没有触发，因为同步完成后已经没有待处理内容。

## 待验证

TODO(verify): watcher 路径的 CDP 重启和注入还没有在最终实现上验证，正式安装包里的 watcher 完整链路也没有运行过。原因：01:01 的测试进程缺 rustls crypto provider，在注入时 panic；01:22 补验时 Codex 正在执行任务，不能重启；23:33 走的是“重启 Codex”路径。触发条件：安装 v6.0.0 或更新版本后，下一次在 API 模式和订阅模式之间切换（使会话 provider 与目标不一致）并打开 Codex。检查：codex-switch 只发出一次针对 `ChatGPT.exe` 根进程的 `taskkill /F /T`；打开后约 40 秒内 Codex 只被重启一次，新的 `ChatGPT.exe` 命令行含 `--remote-debugging-port` 并持续运行（CDP 注入失败时 `launch_codex_with_cdp_hooks` 会结束新进程）；`state_5.sqlite` 中 `threads.model_provider` 全部等于目标 provider。出错时看数据目录下 `logs/codex-switch-errors.jsonl` 里的 `codex_app_process_kill_error`、`codex_app_watcher_on_open_error`。判据不成立时，从该 taskkill 的原始输出和存活 PID 继续定位。

TODO(verify): 会话文件里残留的旧 provider 是否有影响，还不知道。事实：数据库的同步是全量的，会话文件只改写最近活动的 50 个（`SESSION_SYNC_RECENT_ROLLOUT_LIMIT`）。前后两次同步的“最近 50 个”范围不同，2026-09-17 晚同步回 `openai` 之后，仍有 16 个会话（最后更新在 09-14 21:10 到 09-15 21:26 之间）的文件首行 `session_meta.payload.model_provider` 是 `api`，而数据库里是 `openai`。未知：Codex 恢复会话时取的是数据库还是文件里的值。触发条件：在订阅模式下打开这个时间段内的任意一个会话。通过判据：能正常继续对话，说明文件里的旧标记无害，删除本条并在这里记下结果。判据不成立（仍报 “Model provider `api` not found”）时，从 `codex_sessions/rollouts.rs` 的 `collect_recent_rollout_files_from_dirs` 继续：文件同步的范围需要覆盖所有被上一次同步改写过的会话，而不只是当前最近的 50 个。

## 回退

revert 对应的修复 commit 后重新构建。正式安装文件不会被开发构建改动。
