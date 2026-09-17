# 独立 Dev 预览

- Debug build 配合 CODEX_SWITCH_DEV_PREVIEW=1 使用；Release build 忽略该变量。
- 启动前给子进程设置独立 USERPROFILE、APPDATA、LOCALAPPDATA；Tauri overlay config 使用不同 identifier 和窗口标题。
- 预览模式不自动同步自启动、配置、账号、会话，不启动 watcher/账号刷新线程；手动界面 command 保持原实现，在独立数据目录运行。
- autostart 插件名称为 Codex Switch Dev，避免手动设置时影响正式版的开机启动项。
- 2026-09-06：cargo check 通过；完整 UI 启动与配置隔离验证记录在整合验收文档中。

## 隔离环境下的真实运行检查

用途：大改之后，在不碰正式数据、不弹窗口的前提下，让真实后端和真实前端完整跑一遍。

做法：

- Debug 构建时用 `TAURI_CONFIG` 环境变量覆盖三处：`identifier`（避开单实例插件）、主窗口 `visible: false`、`build.devUrl` 指向本地的检查服务。检查服务提供 `renderer/dist` 的副本，并在 `index.html` 里多挂一个检查脚本，脚本通过 `window.__TAURI_INTERNALS__.invoke` 调真实命令，把结果同源 POST 回检查服务。
- 启动进程前设置独立的 `USERPROFILE`、`HOME`、`APPDATA`、`LOCALAPPDATA`、`WEBVIEW2_USER_DATA_FOLDER`，以及 `CODEX_SWITCH_DEV_PREVIEW=1`。
- 沙盒 `.codex` 里放最近若干个 rollout 和 `state_5.sqlite` 的一致性备份（SQLite backup API），并把 `threads.rollout_path` 改写到沙盒目录、删掉没有复制文件的行。不改写的话扫描会报“path 不属于当前 CODEX_HOME”，这是应用的预期保护。不复制 `auth.json`、`config.toml`、`accounts.json`。
- 检查脚本第一步核对 `get_data_dir` 落在沙盒内，不在就什么都不做。
- 不调用会操作真实进程、桌面或外部服务的命令：`restart_current_codex_app_normal`、`open_codex_app_instance`、`show_codex_app_instance`、`switch_account`、`switch_api_mode`、`set_codex_remote_control_enabled`、`open_data_dir`、`open_external_url`、`copy_text`、`oauth_start`、导入导出（会弹文件对话框）。进程枚举是全局的，不受沙盒目录限制。
- API 测试指向检查服务自己的 `/v1`，不出本机。

已知限制（WebView2 153）：`--remote-debugging-port` 参数能传进去但端口不会打开；页面对 `127.0.0.1` 的跨源请求被静默拦截。所以让前端与检查服务同源。另外 `tauri build --debug` 下改 `frontendDist` 不会重新内嵌资源，不要走这条路。

### 2026-09-17 记录（v6.0.0 发版前，代码为提交 `6a7c7e0`）

真实运行，39 项全部通过：

- 前端在隐藏窗口内启动，五个页面（账号、API、Codex、会话、设置）都能渲染，页面无未捕获异常、无 `console.error`，后端日志无 `*_error` 事件。
- 命令：`get_data_dir`、`get_app_version`、`get_settings`、`get_store`、`get_refresh_all_status`、`get_current_codex_app_processes`、`get_codex_app_instance_status`、`get_codex_remote_control_status`、`get_dev_log_entries`、`list_brand_voice_files`、`check_update`、`usage_stats_get`（两次结果相同）。
- 设置写入后读回一致，沙盒 `settings.json` 同步变化。
- 会话管理（12 个真实会话的副本）：扫描、预览、归档后取消归档（列表复原）、删除进回收站（原文件消失）、预览已删除、恢复（列表与文件复原）、再删除后清除。
- `set_codex_proxy_env_enabled` 开和关（沙盒 `.env` 出现和消失）、`set_codex_model_instructions_enabled` 开和关（沙盒 `config.toml` 增减 `model_instructions_file`）。
- `test_api_base_url` 经真实 HTTP 栈访问本机替身服务，收到 `GET /v1/models` 和 `POST /v1/responses`。
- 运行前后正式环境的 `settings.json`、`accounts.json`、`config.toml`、`.env`、`auth.json` 大小和修改时间均未变化。

同一天的两项静态检查：

- `main.rs` 注册的命令 55 个，与 v5.4.12 的集合相同，也与前端 `desktopApi.js` 命令表的 55 个一一对应。
- 把 v5.4.12 与当前后端的每个代码单元（函数、类型、常量、impl 内的方法）去掉注释、可见性、路径前缀后比较：1581 个里 1558 个逐字相同，只是换了位置；变化的 17 个、删除的 2 个、新增的 28 个都对应有意的改动（增量读取、统计缓存、参数下传、重名函数改名）。`updater.rs` 的 24 个单元全部相同。

没有覆盖：需要真实凭据的账号与额度命令、自动更新的下载与安装（debug 构建直接返回）、watcher 接管并重启 Codex、macOS。
