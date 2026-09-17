# 2026-09-06 整合验收记录

一次性记录，对应整合版本 `087a306`。各功能现在的行为看对应文档，这里只保留当时的结论和验证边界。

## 包含的变更

1. 默认 API 测试模型：前端和 Rust 统一为 `gpt-6-astra`，显式填写的模型不受影响。验证：`cargo test api_test_model_tests`，以及前端 normalize 函数的缺省、空白、自定义三种输入；没有向账号 API 发送计费请求。
2. 模型指令文件的保留与覆盖确认、改用原生插件、设置状态反馈：见 [codex-settings.md](codex-settings.md)。
3. Codex 重启流程（异步调度、结束超时、错误传播、1500ms 存活确认）：见 [codex-restart.md](codex-restart.md)。
4. 隔离的 Dev 预览：见 [dev-preview.md](dev-preview.md)。

## 证据

- 本地 Windows：`cargo test` 229 passed、1 ignored；`cargo fmt --check`、`cargo clippy -- -D warnings`、`npm run check` 通过；三个 Node 回归脚本共 11 passed。
- CI run [34031186378](https://github.com/wen495033653/codex-switch/actions/runs/34031186378)：renderer 和 Windows 通过，macOS 有 1 项失败（退出状态 Debug 格式的平台差异）。这个 run 不能记为通过，修复见 codex-restart.md。
- Dev 预览真实启动并加载了界面；启动前后正式版 `settings.json`、本机 Codex `config.toml` 和 `gpt-unrestricted.md` 逐字节一致，正式安装文件未改动。
- 订阅模式下原生 `plugin/list` 成功，`marketplaceLoadErrors=[]`。

## 没有验证的

- 重启测试用的是独立子进程，没有重启真实的 Codex；1500ms 确认只证明进程存活，不代表窗口或业务就绪。
- 没有在真实界面点击“覆盖”，没有动过用户真实的指令文件。
- 代理只验证了文件写入和界面反馈，不代表网络连通；远控没有重新登录或建连。
- API 模式下插件的安装与调用没有端到端验证。
- 历史上的一次 Application Hang 没有 dump。本次只修复了已证实的阻塞调用，不声称找到了那次卡死的全部原因。
- 正式版的构建、签名和发布，以对应 tag 的 Release Workflow 和发布资产为准。
