# Codex Switch 优化整合验收（2026-09-06）

## 最终变更

1. 默认 API 测试模型：前端和 Rust 统一为 `gpt-6-astra`，保留显式指定的模型。
2. 本地模型指令：启动保留已有 `gpt-unrestricted.md`；手动开启时确认保留或覆盖，覆盖前备份。详见 `local-model-instructions.md`。
3. Plugin：移除额外开关、注入及专用重启命令，使用原生目录；保留会话同步、mobile no-replace 所需 CDP。详见 `plugin-cdp.md`。
4. 重启：统一 async command、后台阻塞调度、结束命令超时、完整错误传播及 1500ms 新进程存活确认。详见 `codex-restart.md`。
5. 设置：代理区分已保存配置与实际连接，远控区分模式限制、账号缺失及登录过期。详见 `settings-status.md`。
6. Dev：显式 Debug 预览使用隔离目录，不自动同步自启动、账号、配置或会话。详见 `dev-preview.md`。

## 已观察证据

- 整合版本 `087a306`：本地 Windows `cargo test` 为 229 passed、1 ignored，`cargo fmt --check`、`cargo clippy -- -D warnings`、`npm run check` 通过。
- CI run [34031186378](https://github.com/wen495033653/codex-switch/actions/runs/34031186378)：renderer 和 Windows 通过；macOS 为 226 passed、1 failed、1 ignored。失败定位为退出状态 Debug 格式的平台差异，修复及回归入口见 `codex-restart.md`；不能将该 run 记为通过。
- 修复后的本地 Windows：`cargo test` 为 229 passed、1 ignored（含 8 项 process_control 测试）；`cargo fmt --check`、`cargo clippy -- -D warnings`、`npm run check` 通过。三个 Node 行为回归脚本共 11 passed，显式验证退出码、确认选择、错误传播及设置状态。
- Dev 预览真实启动并加载 UI，进程 Responding=true；启动前后正式版 settings.json、本机 Codex config.toml 和 gpt-unrestricted.md 逐字节一致，正式安装文件未改动。
- 当前订阅模式原生 `plugin/list` 成功，旧 Hook 的 `patched=false`，marketplaceLoadErrors=[]。没有据此声称已完成 API 模式安装、调用的端到端验证。

## 验证边界

- 重启测试使用独立子进程；没有结束或重启承载本任务的 Codex。1500ms 确认仅证明进程存活，不证明窗口或业务请求就绪。
- MD 文件操作和确认分支已有临时目录及 hook 回归；未覆盖真实用户 MD，未执行真实界面的覆盖点击。
- 代理验证覆盖隔离文件写入与反馈，不代表代理外网连接；远控未重新登录或实际建连。
- Dev 使用独立空账号库，未复制登录 token；Debug 预览空闲采样不代表正式版性能 A/B 测试。
- 历史 Application Hang 没有 dump；本次修复已证实的阻塞调用，不声称确定了那一次卡死的全部原因。
- 正式版本构建、签名和发布结果以对应 tag 的 Release Workflow 及发布资产为准；本地构建和旧版本进程不代表新版本安装验收。
