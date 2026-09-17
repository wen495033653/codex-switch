# Codex Switch

[简体中文](./README.md)

Codex Switch is a local desktop tool for managing multiple Codex subscription accounts and switching between subscription accounts and OpenAI-compatible API mode with one click. It runs on Windows and macOS.

![Codex Switch home screen](./docs/images/home.png)

## Download

Download the latest version from [GitHub Releases](https://github.com/wen495033653/codex-switch/releases); later versions can be installed from inside the app. See the [code signing policy](./CODE_SIGNING.md) for release signing details.

## Features

- **Accounts**: add accounts with OAuth, a `refresh_token`, or a JSON file; see quota, subscription expiry and refresh time; scheduled refresh, import and export.
- **API mode**: keep several OpenAI-compatible API profiles, normalize the Base URL automatically, test the connection, and switch to and from subscription accounts with one click.
- **Session sync**: subscription mode and API mode share one session list, so earlier conversations stay usable after switching.
- **Session manager**: browse and preview local sessions; archive, delete (restorable), import and export them.
- **Usage statistics**: token usage per account and API profile, with an estimated cost.
- **Codex proxy**: set a local HTTP/HTTPS proxy for the Codex app; turning it off removes the configuration.
- **Also**: in-app updates, light and dark themes, Chinese and English UI, start at login, a support entry.

All account data and settings stay on your device.

## Contributing

The project is built with Tauri 2, React and Rust.

```bash
npm ci
npm run dev
```

Requirements, project layout and the pull request flow are in [CONTRIBUTING.md](./CONTRIBUTING.md) (Chinese). Issues and pull requests are welcome.

## License

[MIT](./LICENSE)
