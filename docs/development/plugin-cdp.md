# Plugin CDP 兼容性

- 2026-09-06：当前 Codex 26.901.6511.0 资源已改为 app-initial bundle + mcp-request / plugin/list，旧 use-host-config / list-plugins Hook 的状态为 patched=false、attempts=40。
- 当前实现从已加载 app-initial 模块按接口找到唯一消息分发器，只扩展 local + vertical 的 plugin/list 目录查询。其它方法、显式单目录查询、hostId/请求 ID/调度字段保持不变；stop 恢复原分发器。
- CDP 注入等待 Promise，检查 exceptionDetails 和 patched=true；主窗口选择排除 avatar-overlay，不再把传输成功当 Hook 成功。
- 验证：Node --experimental-vm-modules --test scripts/test-plugin-hook.mjs，4 项行为回归通过；Rust plugins::tests 7 项通过。
- 真实运行：在当前 Codex 主窗口注入，version=8、patched=true、attempts=1；实际 plugin/list 返回 error=null、marketplaceLoadErrors=[]，3 个目录，插件数量分别为 5、8、3514。没有执行安装/卸载。
- 原生 CDP 路径：设置 CODEX_SWITCH_TEST_CDP_PORT 后运行 cargo test live_plugin_hook_injection -- --ignored，确认 Rust 注入链路读取 patched=true。
- 回滚：运行 window.__codexSwitchPluginUnlockController.stop() 撤销当前页面 Hook；旧版代码保留在 main，未修改 Codex 安装包。
