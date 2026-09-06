# 独立 Dev 预览

- Debug build 配合 CODEX_SWITCH_DEV_PREVIEW=1 使用；Release build 忽略该变量。
- 启动前给子进程设置独立 USERPROFILE、APPDATA、LOCALAPPDATA；Tauri overlay config 使用不同 identifier 和窗口标题。
- 预览模式不自动同步自启动、配置、账号、会话，不启动 watcher/账号刷新线程；手动界面 command 保持原实现，在独立数据目录运行。
- autostart 插件名称为 Codex Switch Dev，避免手动设置时影响正式版的开机启动项。
- 2026-09-06：cargo check 通过；完整 UI 启动与配置隔离验证记录在整合验收文档中。
