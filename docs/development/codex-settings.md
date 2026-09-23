# Codex 页的设置：代理、远程控制、模型指令、原生插件

## 代理与远程控制的状态反馈（2026-09-06）

- 代理开关显示的是配置状态（已配置 / 未配置），不冒充连接状态。保存或开关返回 `restartRequired`，前端弹出重启 Codex 的提示。常驻说明只有一句“设置 Codex 使用的代理地址”。
- 远程控制只在 API 模式可用。订阅模式下只显示标题、一个“仅 API 模式”标记和开关，不展开说明、连接状态或账号选择。
- API 模式下保留一条用途说明；账号未选、已移除、登录失效只在账号选择处提示，合法账号才显示连接状态徽标。已保存为开启的开关始终允许关闭。
- 远控 helper 更新失败时保留并显示实际错误。
- 验证：`scripts/test-settings-status.mjs`、`scripts/test-remote-control-hints.mjs`（真实组件的静态渲染，中英文 × 各种账号状态，使用虚构账号）；隔离目录里真实执行代理 command：开启和关闭返回 `restartRequired`，保留 `.env` 里的其它变量，空地址显式失败。未验证真实代理连通、远控登录和桌面点击。

## 读取失败不再当成“未安装 / 未应用”（2026-09-23）

- ChatGPT Desktop 安装检测：注册表 `HKCU\...\AppModel\Repository\Packages` 只有“键不存在”算没有安装包；打开或枚举失败返回带 API 名和错误码的错误。已有运行中的 ChatGPT 主进程时不查注册表。查不出来时 `codex_desktop_support_status` 返回 `status: "lookup_failed"`、`supported: false`、`requiresUpdate: false`，`message` 与 `lookupError` 是原始错误；以前返回“未找到新版 ChatGPT Desktop”。多开取可执行文件和会话管理取 CLI 路径时同样返回该错误。
- 多开实例状态：`codex-switch-instance.json` 读不了或解析失败时，该实例仍按目录名给出 key、类型和运行状态，另带 `markerError`；以前静默当成空标记。界面目前不显示 `markerError`。
- 远程控制“配置已应用 / 存在”的判断：没选账号、账号已删除、`auth.json` 不存在、`model_provider` 不是 `api` 仍是“否”；`accounts.json`、`auth.json`、`config.toml` 读取或解析失败改为返回错误。状态轮询因此显示错误而不是“重启 Codex 后生效”；重启和同步流程在应用配置前就返回该错误。`auth.json` 解析失败以前会被当成“未应用”再被覆盖写入，现在要先处理这个错误。
- 显示多开窗口：激活失败记 `codex_app_multi_open_show_window_error`。macOS 上 `osascript` 失败时返回每个 PID 的退出码和 stderr；删掉了其后的 `open -a ChatGPT/Codex` 兜底，它激活的是默认实例而不是这个实例，却按成功返回。
- 远程控制桌面状态查询：响应体读取失败以前被当成空字符串，表现为“HTTP 500 body: ”或 JSON 解析错误；现在返回“读取 ChatGPT 桌面状态响应失败: HTTP 状态码: 原始错误”，经已有的 `lookup_failed` 显示为“桌面状态查询失败”，原文在 `raw`。请求地址写死为 chatgpt.com，没有可注入的测试入口，只过了编译和 Clippy。
- 日志：设置页每 3–4 秒轮询这些状态，同一个事件加同样的内容在一次运行里只写一次（`log_session_sync_event_once`）：`codex_desktop_install_lookup_error`、`codex_app_instance_marker_error`、`codex_remote_control_status_error`。
- 验证：`install_lookup_error_is_reported_unless_a_running_host_decides`（运行中主进程优先、三种查询失败、全部为空仍是 missing）、`package_registry_lookup_reads_the_current_user_key`（本机只读，读到 1 个 `OpenAI.Codex_*` 包，与 PowerShell 读注册表一致）、`unreadable_instance_marker_is_reported_with_the_instance`（临时目录真实文件）、`polled_failure_is_logged_once_per_distinct_details`。远程控制判断会读正式的 `accounts.json`、`auth.json`、`config.toml`，按要求没有写单元测试，只过了编译和 Clippy。macOS 分支本地编不了，以 CI `Check Tauri (macos-latest)` 为准。没有真实运行界面。

TODO(verify): 上述错误在界面上的表现没有真实运行过。触发条件：下一次按 [dev-preview.md](dev-preview.md) 做隔离真实运行时，在沙盒里放一个内容为 `{` 的 `auth.json` 并开启远程控制，另建一个标记文件损坏的多开实例目录；或数据目录 `logs/codex-switch-errors.jsonl` 出现 `codex_desktop_install_lookup_error`、`codex_app_instance_marker_error`、`codex_remote_control_status_error`。通过判据：远程控制状态显示“解析 auth.json 失败…”而不是“重启 Codex 后生效”；`get_codex_app_instance_status` 返回该实例并带 `markerError`；每种错误在 JSONL 里只出现一次。判据不成立时，从 `remote_control.rs` 的 `get_codex_remote_control_status` 和 `session_sync_diagnostics.rs` 的 `log_session_sync_event_once` 继续。

## 模型指令文件 `gpt-unrestricted.md`（2026-09-06）

- 启动时：本地已有文件就直接使用，不提示、不覆盖；缺失时从安装包初始化。
- 手动开启且本地已有文件：后端先返回 `confirmationRequired` 和路径，此时不写任何文件和设置。前端弹窗提供“保留本地并启用”（默认焦点）、“覆盖并启用”、“取消”。
- 保留：继续用本地文件，不读安装包，不建备份。覆盖：先把原文件完整备份为 `gpt-unrestricted.md.backup-<随机标识>` 再写入，并把备份路径告诉用户。取消或点遮罩：不发第二次请求，开关不变。
- 覆盖失败：显示实际错误并保留弹窗；写入失败时用原始字节尝试还原，错误信息包含备份位置和还原结果。
- 界面上该功能的说明文字是“替换instruction.md”，只是文案，实际文件名不变。
- 还原：把备份文件复制回同目录的 `gpt-unrestricted.md`。备份不会被自动删除。
- 验证：`cargo test model_instructions::tests`（临时目录真实读写：字节保留、首次初始化、确认前零写入、保留、覆盖与备份一致）；`scripts/test-model-instructions-confirm.mjs`（弹窗前状态不变、取消不发送、两种选择、失败保留弹窗）；Vite SSR 静态渲染检查中英文文案。没有在真实界面点击覆盖，没有动过本机正在使用的文件。

## 原生插件与 CDP（2026-09-06）

- 删除了“Plugin 增强”开关、它的设置字段和对 `plugin/list` 请求的 Hook，改用 Codex 原生插件目录。旧设置字段 `codex_plugins_enabled` 在读取时丢弃，不再触发重启或注入。
- 依据：[官方插件文档](https://learn.chatgpt.com/zh-Hans/docs/plugins)说明 API key 登录可以管理受支持的官方插件；本机 26.901.6511.0 实测原生 `plugin/list` 无错误（三个目录分别有 5、8、3514 个插件），当时 Hook 并未生效（`patched=false`），说明它已经没有必要。部分 OAuth 插件有限制，任意自定义 API 提供方并未全部验证。
- 独立的插件重启 command 已删除。模型指令、代理、远控改动后的重启提示统一调用 `restart_current_codex_app_normal`。
- `cdp.rs` 只保留两个用途：远控的 `mobile_no_replace` Hook，以及会话同步需要的 CDP 重启流程。
- 验证：Rust 测试（旧字段被移除且不触发动作；会话同步仍按需保留 mobile Hook）；`scripts/test-native-plugins.mjs`（保存设置不再探测插件状态；重启提示成功关闭、失败保留并显示错误；源码里没有旧开关和旧 command）。没有实测 API 身份下的插件安装卸载和远控连接。
