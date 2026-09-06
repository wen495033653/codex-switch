# Codex 设置状态反馈

- 2026-09-06：代理开关显示配置状态，不冒充连接状态；保存/开关返回 restartRequired，前端弹出重启提示。远控 helper 更新失败保留实际错误并提示。
- Plugin 开启和关闭都提示需要重启，避免关闭开关后旧 Hook 仍存在却没有提示。
- 远控同时展示模式、未选账号/已移除账号、登录过期原因；不改账号、不尝试登录、不启用远控。
- 验证：node --test scripts/test-settings-status.mjs 3 项通过；npm run check（语法、敏感信息、i18n、renderer build）通过。
- 隔离目录真实执行代理 command：启用/关闭返回 restartRequired，保留其它环境变量，空地址显式失败。没有修改本机 Codex 的 .env。
- 验证层级：状态函数、实际文件写入与 command 返回；未验证真实代理网络连通或远控登录。
