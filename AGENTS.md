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

## 版本号规则
- 版本统一使用三位：`X.Y.Z`（例如 `1.0.0`、`1.1.0`）。
- 如果你手动改了 `package.json.version`，发布时使用你手动改后的版本。
- 如果你没有手动改，`push` 前自动递进 `minor`（例如 `1.0.0 -> 1.1.0`）。
