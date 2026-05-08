## 构建安装包
在 PowerShell 执行：

```powershell
$env:CSC_IDENTITY_AUTO_DISCOVERY='false'
npm run dist
```

构建产物：
- Windows / macOS 安装包
- Tauri updater artifact
- `latest.json`

## 发布更新

在线更新基于 GitHub Releases 与 `tauri-plugin-updater`。

默认发布流程：

```powershell
git tag v<version>
git push origin v<version>
```

GitHub Actions 会构建 Windows / macOS 安装包并发布 Release metadata。

## Release notes 规则

- 每个发布 tag 必须对应一个同名 release notes 文件，路径固定为 `.github/release-notes/<tag>.md`，例如 `v5.1.0` 对应 `.github/release-notes/v5.1.0.md`。
- 发布前先写好该版本的用户可读更新内容，不要沿用旧版本文案，也不要保留占位句。
- Release notes 只写用户能感知的变化：修复、功能变更、兼容性、安装包或更新行为；实现细节只在确实影响用户时才写。
- 内容保持简洁，优先 3 到 6 条要点。
- 如果是 prerelease，文件名也必须跟 tag 完全一致，例如 `v5.1.0-rc.1.md`。
- workflow 会在发布前读取对应文件；缺文件时应直接失败，不要自动生成默认文案。

## 版本号规则
- 版本统一使用三位：`X.Y.Z`（例如 `1.0.0`、`1.1.0`）。
- 如果你手动改了 `package.json.version`，发布时使用你手动改后的版本。
- 如果你没有手动改，`push` 前自动递进 `minor`（例如 `1.0.0 -> 1.1.0`）。
