# Codex 重启调度

## 流程

- 统一的 Tauri command 使用 async + spawn_blocking，进程等待不占用调用线程。
- 结束前校验可执行路径。taskkill 最长执行 10000ms，超时结束命令；清理最多 1000ms，stdout/stderr 各最多等 500ms。后续目标退出等待上限 12000ms。
- stdout/stderr 并发读取，避免管道写满。失败返回 PID、退出状态、独立的 exitCode、elapsedMs、两路输出和清理结果；非 UTF-8 输出用 rawBase64 保留原字节。所有结束进程调用方传递 Result，不再忽略错误。
- 普通 Codex 启动检查同一个新建 Child 句柄持续存活 1500ms；提前退出（包括 exitCode=0）返回 PID、退出码和耗时，不重试伪装成功。CDP 模式仍使用实际 CDP 注入验证。
- 该确认只证明进程存活，不代表窗口或业务请求已就绪。
- 相关日志：codex_app_process_kill_error/finish、codex_app_launch_confirmation_error/finish、codex_app_restart_command_error。复用现有调试日志入口，错误在 Release 也保留。

## 验证（2026-09-06）

- cargo test process_control::tests：8 项通过。覆盖真实独立子进程的超时与清理、退出码与两路输出、大量输出不阻塞、提前退出 0/23、存活确认后实际 taskkill、错误返回与日志字段。
- 原有 restart_tests 验证后台线程调度及错误传播；全量检查在整合分支执行。
- cargo clippy -- -D warnings：通过。
- 未结束或重启承载本任务的 Codex，没有据测试声称历史卡死根因已全部查明。

## 回退

在验证分支 revert 对应修复 commit，再构建 Dev；正式安装文件未改动。

## 跨平台退出码诊断（2026-09-06）

- CI run 34031186378 的 macOS Rust 测试记录 `ExitStatus(unix_wait_status(5888))`，子进程实际退出码为 23。原错误文本仅输出平台相关的 Debug 原始状态，导致查找退出码的回归断言失败；存活确认后结束子进程测试通过。
- 诊断现在同时保留原始 status 和由 `ExitStatus::code()` 得到的 `exitCode=Some(23)`；命令超时没有已确认退出码时显式记录 `exitCode=None`。不改变结束进程或重启流程。
- 回归断言匹配完整 exitCode 字段，而非任意包含数字 23 的文本。验证入口：`cargo test process_control::tests`，平台结果以对应 CI run 为准。

## taskkill 128 中断重启（2026-09-17）

### 现象与根因

- 切到 API 模式后，`state_5.sqlite` 的 1605 个 thread 仍是 `model_provider=openai`，watcher 每次看到 Codex 打开都判定会话同步待处理，走"结束 → 同步 → CDP 重启"。
- `taskkill /F /T` 结束进程树时，部分后代（如 app-server 下的 `cmd /c` 包装进程）在其子进程被终止后自行退出，taskkill 轮到它们时报"没有此任务的实例在运行"，整体 exit 128。单独 kill 已随主进程退出的 helper PID 同样返回 128。
- `kill_process_tree` 以 taskkill 退出码作为成功判据，`relaunch_running_codex_processes` 的 `?` 随即返回：Codex 已被结束，但会话同步与重启都被跳过，数据库仍是 `openai`，下次打开再次循环，表现为 Codex 启动约 10 秒后闪退。
- 现场证据：外部监控记录 codex-switch 发出 `taskkill /F /T /PID <ChatGPT 主进程>` exit 0 后，对 helper PID 的 taskkill exit 128，之后无重启；Codex 日志无退出记录、无 Crashpad dump；直接调用修复前的 `handle_codex_app_open` 得到 `taskkill /F /T /PID 37024 ... exitCode=Some(128)`，解码输出为 28 条"成功"与 2 条"没有此任务的实例在运行"。

### 实现（唯一实现，不保留按退出码判定的旧逻辑）

- `kill_process_tree`：先快照目标进程树（根 + 全部后代），执行终止（Windows `taskkill /F /T`，其他平台逐个 kill），再等待快照内进程退出（上限 5000ms）。成功只由"进程树全部退出"判定，两个平台一致；终止命令的返回值只作为依据：失败但树已退出时记录 `codex_app_process_kill_tree_exited`（含 `treePids`、`terminationError` 原文），仍有存活则返回错误，附存活 PID、`treePids` 和终止结果。
- `kill_root_process_trees`：重启 Codex（`relaunch_running_codex_processes`）与重开编辑器（`restart_from_ide_snapshot`）只结束根进程（父进程不在同一应用进程集合内），后代随根进程树结束；随后仍等待全部 PID 退出。修复过程中实测逐个结束 Electron helper 会让主进程重新拉起 helper，9 次 taskkill 约 9 秒后才轮到主进程。
- watcher 的根进程识别与上述调用共用 `root_pids`。

### 验证

- 离线：`cargo fmt --check`、`cargo test`（243 passed / 3 ignored）、`cargo clippy -- -D warnings`、`cargo clippy --tests -- -D warnings` 通过。`taskkill_not_found_for_already_exited_tree_is_success_and_logged` 用真实 taskkill 触发 exit 128 并检查日志；`process_tree_kill_outcome_is_decided_by_tree_exit_only` 覆盖终止成功但仍存活、终止失败但已退出等判定；`root_tree_kill_ends_descendants_without_killing_them_separately` 用真实父子进程确认只结束根进程即可清掉子进程，且子进程未被单独 kill。
- CI：run 35127395827（commit bd788c5，workflow_dispatch）renderer、Tauri windows-latest、Tauri macos-latest 全部通过。
- 真实运行，最终实现（2026-09-17 01:20，暂停正式版 Codex Switch，测试二进制调用 `restart_current_codex_app_normal` 作用于运行中的 Codex）：外部监控只看到一次 `taskkill /F /T /PID 34940`（ChatGPT 根进程），exit 128；日志 `codex_app_process_kill_tree_exited`（terminationError 含 exitCode=Some(128)）→ `codex_app_process_kill_finish terminated=true`；01:20:50 全部 ChatGPT.exe 退出，01:20:52 Normal 模式重新打开（`codex_app_launch_confirmation_finish`、`restartedCount=1`），之后持续运行。
- 真实运行，中间版本（01:01，taskkill 非 0 时才确认进程树退出、调用方仍逐个 kill）：测试二进制调用 `handle_codex_app_open`；对主进程的 taskkill exit 128 后流程继续，会话同步完成（`threads.model_provider` 全部变为 `api`，1596 + 归档 10），01:02:08 以 `--remote-debugging-port=9229` 启动 Codex。该测试进程未安装 rustls crypto provider（`main.rs` 启动时安装），CDP 注入请求在 reqwest 内 panic，因此 CDP 注入与 `relaunch_running_codex_processes` 的完成结果未被验证。
- 未验证：watcher 路径的 CDP 重启 + 注入（01:22 准备补验时 Codex 正在执行任务，未重启）；正式安装包内 watcher 线程完整链路。

TODO(verify): watcher 路径的 CDP 重启 + 注入尚未在最终实现上验证，正式安装包内的 watcher 链路也未运行；原因是 01:01 的测试进程缺 rustls crypto provider 在注入时 panic，01:22 补验时 Codex 正在执行任务不能重启。触发条件：安装包含本修复的版本后，下一次在 API / ChatGPT 模式间切换（使会话 provider 与目标不一致）并打开 Codex。检查：codex-switch 只发出一次针对 `ChatGPT.exe` 根进程的 `taskkill /F /T`；打开后约 40 秒内 Codex 仅被重启一次，新 `ChatGPT.exe` 命令行含 `--remote-debugging-port=9229` 并持续运行（CDP 注入失败时 `launch_codex_with_cdp_hooks` 会结束新进程）；`~/.codex/state_5.sqlite` 中 `threads.model_provider` 全部等于目标 provider。Release 仅在内存缓冲错误事件，需要细节时用 Dev 构建日志窗口看 `codex_app_process_kill_error` / `codex_app_watcher_on_open_error`，或外部监控 codex-switch 发出的 taskkill 退出码。判据不成立时，从该 taskkill 的原始输出和存活 PID 继续定位。
