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
