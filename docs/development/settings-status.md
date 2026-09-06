# Codex 设置状态反馈

- 2026-09-06：代理开关显示配置状态，不冒充连接状态；保存/开关返回 restartRequired，前端弹出重启提示。远控 helper 更新失败保留实际错误并提示。
- 远控提示去重（2026-09-06）：订阅模式只展示标题、单个“仅 API 模式”标记及开关，不展开说明框、连接状态徽标或账号选择。
- API 模式保留一条用途说明；未选/已移除账号及登录失效只在账号选择处展示，合法账号才展示连接状态徽标。移除额外前置条件提示框及其专用 helper，不维护第二份提示来源。
- 保留原有开关与账号选择禁用判断、状态轮询和错误处理；已保存开启的开关仍允许关闭。没有修改后端、账号资料或远控启用状态。
- 验证：`node --experimental-vm-modules --test scripts/test-model-instructions-confirm.mjs scripts/test-native-plugins.mjs scripts/test-settings-status.mjs scripts/test-remote-control-hints.mjs` 共 17 项通过；`npm run check`（语法、敏感信息、i18n、renderer build）通过。
- 新增 8 项真实 React 组件静态渲染回归，覆盖中英文订阅模式、账号缺失/未选/过期、合法 API 账号、开启中状态及保留关闭能力。使用本地虚构账号，不调用 API 或修改真实设置；不把静态渲染记为桌面点击或远控连接验证。
- 隔离目录真实执行代理 command：启用/关闭返回 restartRequired，保留其它环境变量，空地址显式失败。没有修改本机 Codex 的 .env。
- 验证层级：状态函数、实际文件写入与 command 返回；未验证真实代理网络连通或远控登录。
