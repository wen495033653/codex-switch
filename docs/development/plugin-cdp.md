# 原生 Plugin 与 CDP

## 当前实现（2026-09-06）

- 删除 Plugin 增强开关、设置默认值/归一化字段/写入入口，以及 plugin/list 请求 Hook。旧 codex_plugins_enabled 字段在读设置归一化时丢弃，不再触发重启或 CDP 注入。
- 删除独立 Plugin 重启 command；MD、代理、远控配置后的重启提示统一调用 restart_current_codex_app_normal，前端状态改名为 codexRestartNotice。
- CDP 模块改名为 cdp.rs，仅保留原有远控 mobile_no_replace Hook 与会话同步需要的 CDP 流程；不再注入 Plugin 脚本。
- 未修改 Codex 安装包、插件配置或已安装插件。正在运行的 Codex 页面中此前注入的脚本与新代码是不同生命周期，完整清除旧页面注册脚本需下次正常重启 Codex；本任务不自动重启当前对话。

## 决策依据

官方文档：https://learn.chatgpt.com/zh-Hans/docs/plugins

API key 登录支持在 Codex CLI/桌面端管理受支持的 OpenAI curated plugins；部分 OAuth 插件存在限制，并非任意自定义 API 提供方都已验证。

本机 26.901.6511.0 的 renderer 默认远程目录逻辑在 API 身份分支不限定 marketplaceKinds。此前原生 plugin/list 请求实测无错误、无 marketplaceLoadErrors，三个目录插件数为 5、8、3514，当时 Hook 状态 patched=false。因此旧 Hook 能工作不代表仍有必要，本次删除该实现。

## 验证

- Rust：旧字段 true/false/缺失均被归一化移除；旧字段不触发打开动作或重启；会话同步仍按需保留 mobile Hook；脚本列表仅包含请求的 mobile Hook。
- scripts/test-native-plugins.mjs：真实前端 hook 的设置保存不再探测 Plugin 状态；通用重启提示成功关闭、失败保留并显示错误；静态检查无旧开关/command。
- 最终整合分支执行 cargo test、cargo fmt --check、cargo clippy -- -D warnings、npm run check 及 Node 回归。
- 不把上述单元/行为回归表述为本机 API 身份切换、Plugin 安装/卸载或远控连接已经实测。

## 回退

在验证分支 revert 本次移除 commit 后重新构建 Dev；Git 保留源代码，移除的本地脚本已进入 Windows Recycle Bin。
