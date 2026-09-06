# 本地模型指令文件

## 行为

- 正常启动：已有本地 gpt-unrestricted.md 直接使用，不提示、不覆盖；缺失时从安装包初始化。
- 手动开启：已有文件时，后端先返回 confirmationRequired 和路径，不写 MD、config 或开关状态；前端弹窗提供“保留本地并启用”“覆盖并启用”“取消”。
- 保留：继续使用用户文件，不读取安装包，不创建备份。
- 覆盖：用户明确确认后，先读取内置版本并完整备份原文件，再写入；备份名为 gpt-unrestricted.md.backup-<随机标识>，响应/提示提供备份路径。
- 取消/点击遮罩：不提交第二次请求，开关状态不变。覆盖失败：显示实际错误，保留弹窗；写入失败时尝试用原始字节还原，错误包含备份位置和还原结果。
- 默认焦点在“保留本地并启用”，不会默认选择覆盖。
- 功能说明按用户指定显示“替换instruction.md”（英文为“Replace instruction.md”）；仅调整界面文案，不更改实际文件名。检测、保留与覆盖说明仅在已有文件的确认弹窗中展示，不在设置页常驻重复提示。

## 验证（2026-09-06）

- cargo test model_instructions::tests：10 项通过，覆盖本地字节保留、首次初始化、目录/资源错误、确认前零写入、确认保留、确认覆盖和备份字节一致。
- node --experimental-vm-modules --test scripts/test-model-instructions-confirm.mjs：4 项通过，覆盖弹窗前状态不变、取消不发送、两个明确选择、失败弹窗保留和首次启用/关闭。
- npm run check、cargo clippy -- -D warnings：通过。
- 验证层级：临时目录真实读写 + 前端 hook 行为回归；没有覆盖本机正在使用的 MD，也没有修改本机 Codex config.toml。

### 功能说明精简（2026-09-06）

- 仅更新功能说明及英文翻译，不修改文件检测、覆盖确认和开关逻辑。
- 使用 Vite SSR 加载实际 CodexPage / I18nProvider，通过 React renderToStaticMarkup 检查 6 个场景：中英文各覆盖开启、关闭及已有文件弹窗；新说明和开关状态正确，旧说明消失，弹窗仍显示文件路径和确认标题。
- npm run check 和 4 项确认弹窗 hook 回归通过。验证层级为静态渲染、hook 行为和前端构建；未验证桌面点击，未替换已安装的正式版。
- 文案改为“替换instruction.md”后重新执行上述 6 项静态渲染断言、4 项 hook 回归和 npm run check，均通过；中英文文案与确认弹窗分别检查，实际文件名和覆盖逻辑未改。
- 2026-09-06 21:08（UTC+8）：使用独立 dev-tauri.json 构建 Debug / no-bundle，并以 CODEX_SWITCH_DEV_PREVIEW=1、独立 USERPROFILE / APPDATA / LOCALAPPDATA 启动；PID 25684，窗口标题 Codex Switch Dev、Responding=true、WebView2 子进程存在。启动前后正式 settings.json、本机 Codex config.toml 和 gpt-unrestricted.md 逐字节一致；未进行界面点击或覆盖操作。

## 还原

需要恢复被用户确认覆盖的文件时，将响应中的 backupPath 复制回同目录的 gpt-unrestricted.md；备份始终保留。
