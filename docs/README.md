# 文档索引

参与开发只需要看前两份。其余是按功能留的记录，改到对应功能时再查。

| 文档 | 什么时候看 |
| --- | --- |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | 第一次参与：环境、项目结构、提交前检查、PR |
| [module-structure.md](development/module-structure.md) | 要新增或移动代码：模块分层与导入约定 |
| [dev-preview.md](development/dev-preview.md) | 想运行开发版又不影响正式数据；大改之后做真实运行检查 |
| [codex-restart.md](development/codex-restart.md) | 改 Codex 的结束、重启、接管和会话同步流程 |
| [codex-settings.md](development/codex-settings.md) | 改 Codex 页：代理、远程控制、模型指令、原生插件 |
| [subscription-refresh.md](development/subscription-refresh.md) | 改订阅到期日、套餐、重置次数的数据来源；改配额、订阅、token 端点的 HTTP 请求与错误信息 |
| [account-store.md](development/account-store.md) | 改 accounts.json / settings.json 的读写、token 轮换或刷新全部 |
| [usage-stats.md](development/usage-stats.md) | 改 token 用量统计的扫描、写库、计价或汇总 |
| [integration-20260906.md](development/integration-20260906.md) | 2026-09-06 整合验收的一次性记录 |

功能记录统一写三件事：当前行为、关键决策和原因、验证记录。验证记录要分清真实运行、离线检查和未验证；还没完成的验证用 `TODO(verify)` 标出，并写明触发条件和通过判据。
