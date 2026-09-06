# Codex 重启调度

- 2026-09-06：两个重启 Tauri command 改为 async + spawn_blocking；进程等待、会话同步、CDP 注入不再占用调用线程。后台错误记录 command、elapsedMs 和实际 error。
- 结束进程前校验可执行文件仍存在；退出超时返回存活 PID；重启数量为零返回错误，不再报成功。
- 验证：cargo test restart_tests，2 项通过；实际线程 ID 不同，故障注入错误原样返回。
- 验证层级：线程调度/错误路径；没有重启承载本任务的 Codex，未据此宣称历史卡死根因已全部查明。
