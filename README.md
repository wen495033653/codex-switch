# Codex Switch

[简体中文](./README.zh-CN.md) | [Releases](https://github.com/wen495033653/codex-switch/releases)

Codex Switch is a lightweight desktop tool for people who use Codex with more than one local identity. It helps you switch Codex subscription accounts, apply an OpenAI-compatible API profile, launch Codex with a process-level proxy, and keep subscription/API session lists aligned.

> Codex Switch is an independent open-source project. It is not affiliated with OpenAI.

## What It Does

- Manage multiple Codex subscription accounts locally.
- Capture the current `~/.codex/auth.json` account into Codex Switch.
- Import accounts with OAuth or a `refresh_token`.
- Import/export account data through `refresh_token` JSON files.
- Switch between saved subscription accounts and API mode.
- Display plan, account status, quota windows, and refresh one or all accounts.
- Configure an OpenAI-compatible API Base URL and API Key.
- Normalize common API Base URL inputs such as `gpt-pool.com`, `https://gpt-pool.com`, and `https://gpt-pool.com/v1`.
- Optionally keep subscription/API Codex session lists in sync.
- Launch Codex with `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `WS_PROXY`, and `WSS_PROXY` injected into the Codex process.
- Create a desktop shortcut that launches Codex with the same proxy behavior.
- Check, download, and install updates from GitHub Releases.

## Installation

Download the latest installer from [GitHub Releases](https://github.com/wen495033653/codex-switch/releases).

Recommended asset types:

- Windows: x64 NSIS setup `.exe`
- macOS Apple Silicon: `aarch64` `.dmg`
- macOS Intel: `x64` `.dmg`

Users do not need to download `.sig` files manually. They are used by the updater to verify release assets.

## How It Works

Codex Switch writes to the same local Codex files that Codex already uses:

- `~/.codex/auth.json`
- `~/.codex/config.toml`
- `~/.codex/sessions/**/rollout-*.jsonl`
- `~/.codex/state_5.sqlite`

Account data and app settings are stored locally in the app data directory:

- Windows: `%APPDATA%/codex-switch/`
- macOS: `~/Library/Application Support/codex-switch/`

Do not commit your app data directory or `~/.codex` files to a public repository.

## Account Mode

1. Open Codex Switch.
2. Add an account with OAuth or `refresh_token` import.
3. Select a saved account.
4. Codex Switch writes that account into the local Codex auth file.

If Codex or supported IDE windows are already open, Codex Switch can prompt before restarting them.

Accounts can also be exported and imported as JSON. Exported files contain `refresh_token` values, so treat them like credentials.

## API Mode

API mode writes a minimal Codex configuration for an OpenAI-compatible provider:

- `~/.codex/auth.json`: API Key for `auth_mode = apikey`
- `~/.codex/config.toml`: `model_provider = "api"`
- `~/.codex/config.toml`: `[model_providers.api]` with `name`, `base_url`, and `requires_openai_auth`

The provider name defaults to `api` when the API profile is applied.

## Session Sync

Codex subscription mode and API mode can have separate local session lists. When session sync is enabled, Codex Switch updates provider metadata in local Codex session records so the same conversation list can be reused between subscription/API mode.

Session sync does not rewrite `cwd` or workspace paths.

## Codex Proxy Launch

The proxy feature only applies when Codex is launched through Codex Switch or through a shortcut created by Codex Switch.

Codex Switch does not change the system proxy. It launches Codex with proxy-related environment variables:

- `HTTP_PROXY`
- `HTTPS_PROXY`
- `ALL_PROXY`
- `WS_PROXY`
- `WSS_PROXY`

If an API Base URL is configured, Codex Switch can also pass `OPENAI_BASE_URL` to the launched Codex process.

## Limits

- Codex Switch depends on the local Codex file formats and OAuth behavior that Codex currently uses.
- API mode requires an OpenAI-compatible endpoint, but provider compatibility still depends on the provider.
- The proxy feature is process-level only; it does not make unrelated apps use the same proxy.
- Update checks use the GitHub Releases endpoint configured in `src-tauri/tauri.conf.json`.

## Development

Requirements:

- Node.js 22 or later
- Rust stable
- Windows 10/11 or macOS
- WebView2 on Windows
- Xcode Command Line Tools on macOS

Install dependencies:

```bash
npm ci
```

Run the app in development mode:

```bash
npm run dev
```

Build the renderer:

```bash
npm run build:renderer
```

Run checks:

```bash
npm run check
npm run check:tauri
```

Rust checks:

```bash
cd src-tauri
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```

Build installers:

```bash
npm run dist
```

Build output is written under `src-tauri/target/release/bundle/`.

See [CONTRIBUTING.md](./CONTRIBUTING.md) for the full local validation checklist.

## Scripts

| Command | Description |
| --- | --- |
| `npm run dev` | Start Tauri development mode. |
| `npm run dev:renderer` | Start only the Vite renderer. |
| `npm run build:renderer` | Build the React renderer. |
| `npm run check` | Run JavaScript checks, lightweight open-source hygiene checks, and renderer build. |
| `npm run check:tauri` | Run `cargo check` for the Tauri app. |
| `npm run dist` | Build release installers. |

## Release

GitHub Actions builds release assets for Windows and macOS when a `vX.Y.Z` or prerelease tag is pushed.

Current release workflow:

- Windows: NSIS `.exe`
- macOS Apple Silicon: `.dmg`
- macOS Intel: `.dmg`
- Updater metadata: `latest.json`
- Updater signatures: `.sig`

Release signing requires these GitHub repository secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

The private key must never be committed. Forks should generate their own updater key pair and replace the updater public key and release endpoint in `src-tauri/tauri.conf.json`.

Example prerelease:

```bash
git tag v5.0.0-rc.1
git push origin v5.0.0-rc.1
```

## Privacy

Codex Switch is designed as a local desktop utility. It stores account tokens and API keys on your machine because switching Codex accounts requires writing local Codex auth/config files.

Review the code before using builds from forks or third-party releases.

## License

MIT. See [LICENSE](./LICENSE).
