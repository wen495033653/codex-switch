# Codex Switch

[English](./README.md) | [Releases](https://github.com/wen495033653/codex-switch/releases)

Codex Switch 是一个轻量桌面工具，面向需要在本机切换多个 Codex 身份的用户。它可以管理 Codex 订阅账号、应用 OpenAI-compatible API 配置、通过进程级代理启动 Codex，并让订阅/API 模式沿用同一份本地会话列表。

> Codex Switch 是独立开源项目，和 OpenAI 没有关联。

## 功能

- 本地管理多个 Codex 订阅账号。
- 读取当前 `~/.codex/auth.json` 并保存为账号。
- 通过 OAuth 或 `refresh_token` 导入账号。
- 通过包含 `refresh_token` 的 JSON 文件导入/导出账号数据。
- 在订阅账号和 API 模式之间切换。
- 显示套餐、账号状态、quota window，并支持刷新单个或全部账号。
- 保存 OpenAI-compatible API 的 Base URL 和 API Key。
- 自动规范化常见 Base URL 输入，例如 `gpt-pool.com`、`https://gpt-pool.com`、`https://gpt-pool.com/v1`。
- 可选开启订阅/API 会话同步。
- 启动 Codex 时注入 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY`、`WS_PROXY`、`WSS_PROXY`。
- 创建带有相同启动行为的桌面图标。
- 从 GitHub Releases 检查、下载并安装更新。

## 安装

从 [GitHub Releases](https://github.com/wen495033653/codex-switch/releases) 下载对应平台安装包。

推荐下载类型：

- Windows：x64 NSIS setup `.exe`
- macOS Apple Silicon：`aarch64` `.dmg`
- macOS Intel：`x64` `.dmg`

普通用户不需要手动下载 `.sig` 文件。`.sig` 给应用内 updater 校验安装包使用。

## 工作方式

Codex Switch 会写入 Codex 本身使用的本地文件：

- `~/.codex/auth.json`
- `~/.codex/config.toml`
- `~/.codex/sessions/**/rollout-*.jsonl`
- `~/.codex/state_5.sqlite`

账号数据和应用设置保存在本地应用数据目录：

- Windows：`%APPDATA%/codex-switch/`
- macOS：`~/Library/Application Support/codex-switch/`

不要把应用数据目录或 `~/.codex` 下的敏感文件提交到公开仓库。

## 账号模式

1. 打开 Codex Switch。
2. 通过 OAuth 或 `refresh_token` 导入账号。
3. 选择保存好的账号。
4. Codex Switch 会把该账号写入本机 Codex auth 文件。

如果 Codex 或支持的 IDE 窗口已经打开，Codex Switch 可以在切换时提示是否重启。

账号也可以导出和导入为 JSON。导出的文件包含 `refresh_token`，请按 credential 处理。

## API 模式

API 模式会写入一份最小可用的 OpenAI-compatible Codex 配置：

- `~/.codex/auth.json`：写入 `auth_mode = apikey` 使用的 API Key。
- `~/.codex/config.toml`：写入 `model_provider = "api"`。
- `~/.codex/config.toml`：写入 `[model_providers.api]` 的 `name`、`base_url`、`requires_openai_auth`。

应用 API 模式时，provider name 默认使用 `api`。

## 会话同步

Codex 订阅模式和 API 模式默认可能使用不同的本地会话列表。开启会话同步后，Codex Switch 会更新本地 Codex 会话记录中的 provider 元数据，让订阅/API 模式可以沿用同一份会话列表。

会话同步不会改写 `cwd` 或 workspace 路径。

## Codex 代理启动

代理功能只对通过 Codex Switch 启动的 Codex，或通过 Codex Switch 创建的桌面图标启动的 Codex 生效。

Codex Switch 不会修改系统代理。它会在启动 Codex 进程时注入这些代理环境变量：

- `HTTP_PROXY`
- `HTTPS_PROXY`
- `ALL_PROXY`
- `WS_PROXY`
- `WSS_PROXY`

如果已经配置 API Base URL，Codex Switch 也可以向启动的 Codex 进程传递 `OPENAI_BASE_URL`。

## 限制

- Codex Switch 依赖 Codex 当前使用的本地文件格式和 OAuth 行为。
- API 模式要求 OpenAI-compatible endpoint，但具体兼容性仍取决于 provider。
- 代理功能只作用于被 Codex Switch 启动的进程，不会让其他应用自动使用同一代理。
- 更新检查使用 `src-tauri/tauri.conf.json` 中配置的 GitHub Releases endpoint。

## 开发

环境要求：

- Node.js 22 或更高版本
- Rust stable
- Windows 10/11 或 macOS
- Windows 需要 WebView2
- macOS 需要 Xcode Command Line Tools

安装依赖：

```bash
npm ci
```

开发模式启动：

```bash
npm run dev
```

构建 renderer：

```bash
npm run build:renderer
```

基础检查：

```bash
npm run check
npm run check:tauri
```

Rust 检查：

```bash
cd src-tauri
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

构建安装包：

```bash
npm run dist
```

构建产物位于 `src-tauri/target/release/bundle/`。

完整本地验证清单见 [CONTRIBUTING.md](./CONTRIBUTING.md)。

## 脚本

| 命令 | 用途 |
| --- | --- |
| `npm run dev` | 启动 Tauri 开发模式。 |
| `npm run dev:renderer` | 只启动 Vite renderer。 |
| `npm run build:renderer` | 构建 React renderer。 |
| `npm run check` | 执行 JavaScript 检查、轻量开源 hygiene 检查，并构建 renderer。 |
| `npm run check:tauri` | 对 Tauri 应用执行 `cargo check`。 |
| `npm run dist` | 构建 release 安装包。 |

## 发布

推送 `vX.Y.Z` 或 prerelease tag 后，GitHub Actions 会构建 Windows 和 macOS 安装包。

当前 release workflow：

- Windows：NSIS `.exe`
- macOS Apple Silicon：`.dmg`
- macOS Intel：`.dmg`
- updater metadata：`latest.json`
- updater signatures：`.sig`

发布签名需要配置 GitHub repository secrets：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

私钥不能提交到仓库。fork 后如果要发布自己的安装包，建议生成自己的 updater key pair，并同步替换 `src-tauri/tauri.conf.json` 中的 updater 公钥和 release endpoint。

prerelease 示例：

```bash
git tag v5.0.0-rc.1
git push origin v5.0.0-rc.1
```

## 隐私

Codex Switch 是本地桌面工具。由于切换 Codex 账号需要写入本机 Codex auth/config 文件，它会在本机保存账号 token 和 API Key。

使用 fork 或第三方构建前，请先审查代码。

## License

MIT。见 [LICENSE](./LICENSE)。
