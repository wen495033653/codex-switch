# Codex 页的设置：代理、远程控制、模型指令、原生插件

## 远程控制下的订阅额度显示兼容（2026-10-28）

### 范围与数据流

- 用户明确要求：仅在 SW 开启远程控制、保留订阅登录而会话走 API 时，官方订阅额度耗尽不应锁住发送，也不显示对应的“Codex 和工作使用额度已用完”框。普通 API、订阅模式、独立多开实例保持原行为。
- 不改自动审批模型、审批策略或模型目录。前一轮额外加入的 `auto_review_model_override` 及受管 catalog 逻辑已撤销；只读确认正式默认 home 和已有实例均未启用该目录，因此没有为本次撤销改写正式配置。
- 本机 Desktop `26.1023.6123.0`：primary bundle 的官方 quota selector `Yz` 导出 `Kq`，Local adapter 读为 `Hr` 并加入 `xi`，再传 `FK.submitDisabled`。额度显示框来自 composer-content 对官方 quota 的 render-local 数据消费。两处只处理官方额度，不清第三方 API 自己的错误、策略限制、模型加载或审批状态。
- 额度框的实际消费链为 `AM.g → rateLimit → bg.rateLimit → primary.u1r`；`u1r` 只在官方 quota 非空且存在 upsell 条件时创建外框和 `Kz/d1r`。将 render-local `g` 置空会跳过整个官方额度框和其警告分支，直接返回原 lower-priority 内容，不留下空卡片；独立的 image-generation 分支不改。
- 调用方仅在有效远控开启时启动兼容；renderer 同时核实 Local provider 为 SW 的 `api` 且 provider 带远控混合配置的 bearer 字段（只判存在/非空，不记录值）。普通 API 的 `requires_openai_auth=true` 本身不构成远控依据。
- 已用当前 CLI 的隔离 `config/read` 验证 renderer 所读的 `config.model_providers.api.experimental_bearer_token` 仍为非空字符串，未被删成 null；前端 `_g(...).config → ng → cv.data` 保留该字段树。验证仅用虚构凭据，证据为 `%TEMP%/codex-sw-config-read-rk7mpvnq/field-shape.json`，没有读取或输出正式 key。
- 关闭远控、控制账号失效/移除、切回订阅时停止 SW 持有的 quota Hook；不结束 Codex，不修改官方安装包，不新增 Node 生产依赖。日志沿用 `codex_api_quota_hook_ready/stopped/error`，中文说明对应远控额度兼容。
- 已显示的额度状态会被 Maitai/Jotai 缓存，仅挂断点或普通 rerender 不保证立即重算。因此首次生效只允许在 SW 自己启动/重启的新 child 上：先挂发送 URL 断点，再只读 fetch 本地静态 JS 以预挂 banner URL 断点，随后单次 `Page.reload`。延迟加载的 banner 可先 pending，首次执行前由 V8 自动绑定；日志区分发送已绑定与 banner pending，不将后者称为已完成渲染。
- 对已有活跃窗口，首次 attach 返回 `[REMOTE_QUOTA_RESTART_REQUIRED]`，提示通过 SW 的现有“重启 Codex”操作生效，不静默刷新，不把旧缓存当作已修复；同 target 已就绪的 supervisor 可复用，不重复 reload。远控配置预检查/应用失败时传播原错并撤 scope；post-exit 自动关闭远控后使用最新 hook flags，未核实 scope 时不沿用旧开启状态。

### 外部 60 秒提示与原生就绪

- 用户引用的 `ERROR: Codex Input Unlock did not become ready within 60 seconds` 和 `Press Enter to close this window` 来自外部 [Codex Input Unlock PowerShell 启动器](https://github.com/1zero224/codex-input-unlock)。本机默认安装目录不存在，无该脚本/supervisor 进程，SW 源码和已查日志中未出现该原文，未证明本机发生过该故障。
- SW 使用原生 Rust CDP，不启动上述 PS/Node 启动器或等待 Enter 的控制台窗口。定位与就绪必须保留 target、script、locator、actual breakpoint 的原始失败依据；不靠增加等待时长掩盖失败，不把仅有进程或端口当成 UI 已修复。
- 原生 RPC 使用同一总 deadline，无关事件流不延长预算。owned reload 和后续正常刷新按 `Destroyed/Cleared/Created` 等待主 frame 的新 default context，再显式使用其 `contextId`；iframe 的 default context 不解除等待。仅在旧 context 销毁和对应 context-invalid 协议错误均已证实的情况下，将在途只读 evaluate 转到新 context；其他 RPC 错误保留原文且不重发。对应状态变化记录 `codex_api_quota_context_replaced`，包含原 code/message 和 old/new context。
- Ready 发布只允许从 Starting 且未取消时完成；取消不能被迟到的 Ready 覆盖，已存在的 Failed 原错优先保留。缺 selector、缺主 context 或取消都显式结束等待并保留阶段诊断，不让用户只看到无依据的固定 60 秒提示。

### 验证入口与边界

- `scripts/test-api-quota.mjs` 的 opt-in 真实 V8 + Rust supervisor 入口：`CODEX_SWITCH_TEST_CDP=1 node --test scripts/test-api-quota.mjs`。定向覆盖有效远控、普通 API、订阅、官方额度共享对象不变、停止恢复、首次已缓存界面的重算与刷新。
- 最终离线检查：`cargo test` 为 383 passed / 6 ignored；`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 通过；`npm run check` 为 69 passed / 4 opt-in skipped，前端与 hygiene 构建通过。本次 opt-in V8 入口另行显式运行，7 项全部通过。
- 已真实运行（隔离 V8，不是正式 UI）：先缓存 `blocked=true/banner=true`，warm 首次 attach 不刷新；owned 单次 reload 后首次缓存读取 `blocked=false/firstBanner=false`，重复读取不增加重算次数；lazy banner 第一次执行即 false。普通 API/订阅保持原 true；共享官方 quota 全程仍为 `allowed=false` 和 `usage_limit`。缺 selector 在约 37ms 返回原 V8 stack，启动取消约 240ms 返回。
- 已真实运行（本地 CDP 协议 fixture）：持续无关事件仍受总 RPC deadline 约束；主 default context 延迟创建、iframe 先到、正常刷新、缺主 context、在途旧 context 销毁后切换均有定向证据；任意其他 RPC error 不重发。取消后发布 Ready 的确定序列也保持取消。Node 的重编译/生命周期适配器与实际 Chromium 页面导航分开，不将这些证据称为正式 Codex 整机 UI 验证。
- 本机替换（2026-10-28）：`npm run tauri:build -- --no-bundle` 通过；本轮最终 exe 为 9,284,608 字节，构建时间 21:16（UTC+8）。用户确认后只结束旧 SW，等待其进程退出并释放句柄，再复制修复版；复制后长度和修改时间与构建产物一致，原版 `codex-switch.before-api-quota-20261028.exe` 保留。当前 Codex 的 PID 未变，未为替换刷新或重启；SW 按确认保持关闭。回退时退出 SW，用该原版备份覆盖主程序，再重启 Codex 以清除运行期 Hook/缓存。
- 本机正式远控当前关闭；没有为测试切换正式账号、修改正式 `config.toml` 或重启当前 Codex。隔离验证与真实耗尽订阅账号的现场结果分开记录，不将前者称为后者。

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
- 日志：设置页每 3–4 秒轮询这些状态，同一个事件加同样的内容在一次运行里只写一次（`log_event_once`）：`codex_desktop_install_lookup_error`、`codex_app_instance_marker_error`、`codex_remote_control_status_error`。
- 验证：`install_lookup_error_is_reported_unless_a_running_host_decides`（运行中主进程优先、三种查询失败、全部为空仍是 missing）、`package_registry_lookup_reads_the_current_user_key`（本机只读，读到 1 个 `OpenAI.Codex_*` 包，与 PowerShell 读注册表一致）、`unreadable_instance_marker_is_reported_with_the_instance`（临时目录真实文件）、`polled_failure_is_logged_once_per_distinct_details`。远程控制判断会读正式的 `accounts.json`、`auth.json`、`config.toml`，按要求没有写单元测试，只过了编译和 Clippy。macOS 分支本地编不了，以 CI `Check Tauri (macos-latest)` 为准。没有真实运行界面。

TODO(verify): 上述错误在界面上的表现没有真实运行过。触发条件：下一次按 [dev-preview.md](dev-preview.md) 做隔离真实运行时，在沙盒里放一个内容为 `{` 的 `auth.json` 并开启远程控制，另建一个标记文件损坏的多开实例目录；或数据目录 `logs/codex-switch-errors.jsonl` 出现 `codex_desktop_install_lookup_error`、`codex_app_instance_marker_error`、`codex_remote_control_status_error`。通过判据：远程控制状态显示“解析 auth.json 失败…”而不是“重启 Codex 后生效”；`get_codex_app_instance_status` 返回该实例并带 `markerError`；每种错误在 JSONL 里只出现一次。判据不成立时，从 `remote_control.rs` 的 `get_codex_remote_control_status` 和 `app_log.rs` 的 `log_event_once` 继续。

## 模型指令文件 `gpt-unrestricted.md`（2026-09-06）

- 启动时：本地已有文件就直接使用，不提示、不覆盖；缺失时从安装包初始化。
- 手动开启且本地已有文件：后端先返回 `confirmationRequired` 和路径，此时不写任何文件和设置。前端弹窗提供“保留本地并启用”（默认焦点）、“覆盖并启用”、“取消”。
- 保留：继续用本地文件，不读安装包，不建备份。覆盖：先把原文件完整备份为 `gpt-unrestricted.md.backup-<随机标识>` 再写入，并把备份路径告诉用户。取消或点遮罩：不发第二次请求，开关不变。
- 覆盖失败：显示实际错误并保留弹窗；新内容先写到同目录的临时文件再改名覆盖（`write_file_atomically`，2026-10-17 起），写入失败时本地文件不变，错误信息包含备份位置。以前直接 `fs::write`（先截断再写），失败时文件只剩一半，再用原始字节尝试还原。
- 界面上该功能的说明文字是“替换instruction.md”，只是文案，实际文件名不变。
- 还原：把备份文件复制回同目录的 `gpt-unrestricted.md`。备份不会被自动删除。
- 验证：`cargo test model_instructions::tests`（临时目录真实读写：字节保留、首次初始化、确认前零写入、保留、覆盖与备份一致）；`scripts/test-model-instructions-confirm.mjs`（弹窗前状态不变、取消不发送、两种选择、失败保留弹窗）；Vite SSR 静态渲染检查中英文文案。没有在真实界面点击覆盖，没有动过本机正在使用的文件。

## 原生插件与 CDP（2026-09-06）

- 删除了“Plugin 增强”开关、它的设置字段和对 `plugin/list` 请求的 Hook，改用 Codex 原生插件目录。旧设置字段 `codex_plugins_enabled` 在读取时丢弃，不再触发重启或注入。
- 依据：[官方插件文档](https://learn.chatgpt.com/zh-Hans/docs/plugins)说明 API key 登录可以管理受支持的官方插件；本机 26.901.6511.0 实测原生 `plugin/list` 无错误（三个目录分别有 5、8、3514 个插件），当时 Hook 并未生效（`patched=false`），说明它已经没有必要。部分 OAuth 插件有限制，任意自定义 API 提供方并未全部验证。
- 独立的插件重启 command 已删除。模型指令、代理、远控改动后的重启提示统一调用 `restart_current_codex_app_normal`。
- `cdp.rs` 保留远控的 `mobile_no_replace` Hook、会话同步需要的 CDP 重启流程；2026-10-28 增加上文的远程控制订阅额度显示兼容。
- 验证：Rust 测试（旧字段被移除且不触发动作；会话同步仍按需保留 mobile Hook）；`scripts/test-native-plugins.mjs`（保存设置不再探测插件状态；重启提示成功关闭、失败保留并显示错误；源码里没有旧开关和旧 command）。没有实测 API 身份下的插件安装卸载和远控连接。

## 远程控制关闭时不再改动未选模式的配置（2026-10-17）

- 现象（代码审查发现）：`codex_active_mode` 是默认的空值（从没切换过账号或 API）、远程控制关闭、API 页保存过带 Base URL 的配置时，每次启动的 `sync_remote_control_runtime_for_current_settings("app_start")` 都把运行目标判成 API，执行 `set_api_mode`：`auth.json` 被整份换成 API Key，没有存进 Codex Switch 的 ChatGPT 登录随之丢失；Base URL 为空时则删除 `[model_providers.api]`。其它恢复 API 的路径（`restore_api_mode_if_selected`、保存设置）都只在模式为 `api` 时才动手。
- 现在：远程控制关闭时，只有模式为 `api`，或者残留了远控混合配置（它本来就让 Codex 跑在当前 API 配置上）才恢复 API 配置；模式为空时只清理远控配置和旧版残留，不碰 `auth.json` 和 API provider。
- 验证：`api_config_is_restored_only_for_a_selected_api_mode_or_a_leftover_mixed_config`（判断函数的三种情况）。同步本身会读写正式的 `auth.json`、`config.toml`，没有单元测试，也没有真实运行。

## 远程控制开启时保存 API 配置（2026-10-17）

- 现象（代码审查发现）：远程控制生效时，在 API 页保存、新增或删除任意配置，`update_settings` 都会对当前配置执行 `set_api_mode`：`auth.json` 被换成纯 API Key、`[model_providers.api]` 被整表重写，远程控制账号的 ChatGPT token 和 provider 的 bearer 一起消失，设置里却仍显示远程控制已开启，要等 watcher 下一次处理打开 Codex 才恢复。“切换 API”路径在 `set_api_mode` 之后会补一次远控同步，保存设置这条路径没有。
- 现在：远程控制开启时，保存 API 配置改为调用 `sync_remote_control_runtime_for_current_settings("update_settings")`，用当前配置重新应用混合配置；关闭时仍是 `set_api_mode`。同步可能因为远控账号已删除而关闭远程控制，所以之后重新读取设置再返回给界面。
- 验证：`saving_an_api_profile_keeps_the_remote_control_mixed_config`（判断远控、纯 API、非 API 模式、缺 Key 和从 auth.json 取 Key 五种情况）。实际写入会动正式的 `auth.json`、`config.toml`，没有真实运行。

## config.toml 的读写（2026-10-17）

- 表头识别：`[features] # 说明` 这种带行尾注释的表头是合法 TOML，以前只认 `[`…`]` 结尾的整行，带注释的表头被当成普通行：第一张表的表头带注释时，`model_provider = "api"` 等根键会被插到下一张表前面，按 TOML 语义归进了上一张表，Codex 读不到；目标表头带注释时（如 `[model_providers.api] # 中转`）找不到这张表，又追加一个同名表，文件变成非法 TOML。现在所有表头判断（根区结束、整表替换、读取表内容、多开实例的配置）都用 `codex_config::table_header`：`]`（数组表是 `]]`）之后只能是空白或 `#` 注释。
- 只在内容变化时写入：`set_table_config`、`remove_table_config`、`set_config_values`、`remove_config_values`、移除远控键都经过 `update_config_lines`，先在内存里改，行没变就不写；写入时保留文件原有的 CRLF。以前每次启动都会对 `~/.codex/config.toml` 无条件重写 2–4 次（改动修改时间、把 CRLF 改成 LF，并与 Codex 自己写这个文件的时机叠加）。读取时文件不存在按空配置处理，不再顺手创建一个空的 `config.toml`；“打开 config.toml”仍会先创建它。
- 验证（离线）：`table_headers_may_carry_a_trailing_comment`、`headers_with_a_trailing_comment_still_end_the_root_and_name_their_table`、`writing_the_same_values_again_changes_no_line`（整表替换、根键写入、删除不存在的键各执行两次，第二次不产生任何变化）；全部 349 个测试和 Clippy 通过。写盘本身动的是正式的 `config.toml`，没有单元测试。

## 多开与远控的日志（2026-10-17）

- 多开的开始、完成（时间线）、显示窗口及其失败事件以前记 `channel`（含邮箱末 4 位）或完整 `targetId`（订阅账号的 profile id 末尾就是邮箱），失败事件还会同时写进错误日志。按日志约定改为 `target`：订阅账号只记 `account_label` 的前 8 位，API 配置记配置 id。界面收到的返回值不变。
- 远控因登录过期自动关闭时，把账号标记为失效失败的事件以前叫 `codex_remote_control_account_status_update_failed`，不以 `_error` 结尾也不在任何名单里，既不落盘也不显示；改名为 `codex_remote_control_account_status_update_error`，并记账号前 8 位。
- 远控“桌面状态查询失败”以前只变成界面上的 `lookup_failed`，不记日志；现在按每种错误每次运行一条记 `codex_remote_control_backend_lookup_error`。查询的网络错误改用 `accounts::http_error_message` 展开原因链（DNS、TLS、超时等），以前只有一句 “error sending request for url (…)”。
- 验证：只过了编译和 Clippy；这些都是日志字段和事件名的修改，没有真实触发。

## 多开实例的 `.env` 跟随主目录（2026-10-17）

- 打开多开实例时，会把 `~/.codex` 的 `.env` 和 `AGENTS.md` 复制进实例目录。以前主目录里没有这个文件时直接跳过：开代理打开过实例、再关代理（`.env` 只剩代理行时会被删掉），实例里的旧 `.env` 一直保留 `HTTP_PROXY` 等变量，代理软件关掉后只有这个实例连不上网。现在主目录没有该文件时，实例里的副本也删除，删除失败返回错误。
- 验证（离线）：`instance_copy_follows_a_file_removed_from_the_main_home`（临时目录真实文件：复制、主目录删除后实例副本被删、两边都没有时不报错）。
