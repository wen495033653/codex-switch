# Codex Switch

[简体中文](./README.md) | [Releases](https://github.com/wen495033653/codex-switch/releases)

Codex Switch is a local desktop tool for switching Codex subscription accounts, applying an OpenAI-compatible API profile, and managing Codex local proxy settings.

## Features

- Manage multiple Codex subscription accounts.
- Import accounts with OAuth, `refresh_token`, or JSON files.
- Switch between subscription accounts and API mode.
- Save API Base URL and API Key, with normalization for common Base URL inputs.
- Sync local session lists between subscription/API mode.
- Manage Codex local HTTP/HTTPS proxy settings.
- Check, download, and install updates from GitHub Releases.

## Installation

Download the installer for your platform from [GitHub Releases](https://github.com/wen495033653/codex-switch/releases):

- Windows: x64 `.exe`
- macOS Apple Silicon: `aarch64` `.dmg`
- macOS Intel: `x64` `.dmg`

Users do not need to download `.sig` files manually. They are used by the in-app updater to verify release assets.

## Local Files

Codex Switch reads and writes the local files already used by Codex:

- `~/.codex/auth.json`
- `~/.codex/config.toml`
- `~/.codex/sessions/**/rollout-*.jsonl`
- `~/.codex/state_5.sqlite`

Account data and app settings are stored in the app data directory:

- Windows: `%APPDATA%/codex-switch/`
- macOS: `~/Library/Application Support/codex-switch/`

Exported account JSON files contain `refresh_token` values, so treat them like credentials.

## Development

```bash
npm ci
npm run dev
```

Common checks:

```bash
npm run check
npm run check:tauri

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

## Release

GitHub Actions builds Windows and macOS installers when a `vX.Y.Z` or prerelease tag is pushed.

Updater signing requires these GitHub repository secrets:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

The private key must never be committed.

## License

MIT. See [LICENSE](./LICENSE).
