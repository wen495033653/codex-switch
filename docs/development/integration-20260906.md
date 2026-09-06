# Codex Switch 优化整合验收（2026-09-06）

## 变更

1. 默认 API 测试模型：前端和 Rust 统一为 gpt-6-astra，保留显式指定的模型。
2. 本地模型指令：优先使用已有 gpt-unrestricted.md，安装包仅初始化缺失文件，不覆盖用户更新。
3. Plugin：适配当前 app-initial / mcp-request / plugin/list；CDP 检查实际 patched 状态，排除 avatar-overlay。
4. 重启：Tauri command 在后台线程执行；结束进程前检查可执行路径；退出失败包含 PID；重启数为零不报成功。
5. 设置：代理显示“已配置/未配置”并提示重启，远控区分模式限制、账号缺失与登录过期，Plugin 关闭也提示重启。
6. Dev：独立 Debug 预览，不自动同步自启动、账号、配置或会话；正式版没有被覆盖。

## 验证

- cargo fmt --check：通过。
- cargo test：217 passed，2 ignored。两个需要显式目标的测试已另行执行：真实 Codex CDP 注入、隔离目录代理 command，均通过。
- cargo clippy -- -D warnings：通过。
- npm run check：语法、敏感信息、i18n、renderer build 通过。
- node --experimental-vm-modules --test scripts/test-plugin-hook.mjs scripts/test-settings-status.mjs：7 passed。
- 默认模型：前端函数缺省/空白/自定义三种输入通过；Rust 对应测试通过。
- 当前 Codex 实测：Hook version=8、patched=true、attempts=1；plugin/list error=null，marketplaceLoadErrors=[]，目录插件数为 5、8、3514。未执行安装/卸载。
- Dev Debug build（不同 identifier/title）完成；Windows 实际窗口标题 Codex Switch Dev，进程 Responding=true，UI Automation 与截图确认界面正常加载。
- Dev 启动前后：正式版 settings.json、本机 Codex config.toml 和 gpt-unrestricted.md 逐字节一致。正式版进程已关闭，当前对话 Codex 进程未重启。

## 验证边界

- 重启的线程调度及错误传播已测试；没有执行当前对话 Codex 的真实关闭/重启。
- 代理验证覆盖文件写入与反馈，不代表代理外网连接已测试；远控没有重新登录或实际建连。
- Dev 使用独立空账号库，未复制登录 token；不应把该界面当作正式版账号丢失。
- 历史 Application Hang 没有 dump；本次修复已证实的阻塞调用，不声称确定了那一次卡死的全部原因。
- 代码位于 validate/codex-switch-optimizations-20260906，等待界面确认；未合入 main、未打 tag、未发布 Release。
