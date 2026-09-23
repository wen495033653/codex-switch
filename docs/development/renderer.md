# 前端状态与错误处理

按功能记录：当前行为、关键决策和原因、验证记录。

## 渲染检查的做法

前端测试（`scripts/test-*.mjs`）不覆盖页面交互。改页面时另做一次渲染检查：用 `vite --configLoader native --config <临时配置>` 启动前端，临时配置导入项目的 `vite.config.mjs`，再用 `transformIndexHtml` 在 `main.jsx` 之前注入一段脚本，把 `window.api` 换成记录每次调用的替身（`installTauriApiBridge` 发现已有 `window.api` 就不再安装）。替身按 URL 参数返回固定数据或失败，数据只用虚构账号。这样不连接后端，也不碰正式数据。

内置浏览器面板隐藏时 `document.visibilityState` 是 `hidden`：`useAsyncPolling` 会暂停，`requestAnimationFrame` 和 `ResizeObserver` 要等到截图时才执行一帧。检查轮询时让替身把 `visibilityState` 报成 `visible`；检查尺寸测量时，操作后先截一次图再读结果。

## API 预检结果的保存（2026-09-23）

- 预检结果按配置 ID 存在设置的 `api_test_results` 里，只保存已完成的结果。读取时丢弃 `loading` 项，旧版本已经存进去的也一样。
- 某个配置是否正在预检，由 `ApiModePage` 内存里的集合判断，请求结束（成功或失败）时移出。保存结果回流时，正在预检的配置保留内存里的进行中状态，其余配置以保存的结果为准。
- 原因：旧逻辑每次保存整张表，别的配置的 `loading` 项也被写盘；重启后按 `loading` 拦截，这个配置一直显示“预检中”、按钮不可用。
- 已知限制：离开 API 页再回来，页面重新挂载，集合是新的；旧页面发出的请求仍会完成并保存。这段时间里同一配置可以再次发起预检，和修改前相同。
- 验证：
  - 离线：`scripts/test-api-precheck.mjs`（读取丢弃 `loading`；回流时只保留正在进行的配置）。
  - 渲染检查（替身后端，非真实后端）：修改前，配置 2 预检中时配置 1 完成，保存内容含配置 2 的 `loading: true`，重新加载后配置 2 一直“预检中”、按钮禁用、点击不发请求。修改后，同一份旧数据读入时配置 2 正常；同样的操作保存内容只有已完成项，配置 1 保存回流后配置 2 仍显示“预检中”，重复点击不发第二个请求，完成后两项都保存。
  - TODO(verify)：真实应用里还没跑过。触发条件：下次按 [dev-preview.md](dev-preview.md) 做隔离环境真实运行，或下次发版前。做法：两个 API 配置同时预检（其中一个指向慢响应的替身 `/v1`），一个完成后重启应用。查看沙盒 `settings.json` 的 `api_test_results`。通过判据：不含 `"loading": true`，重启后两个配置都能再次预检。不通过时从 `ApiModePage.jsx` 的 `setApiTestForProfile` 和 `utils/apiPrecheck.js` 的 `normalizeApiTestResults` 查起。

## 取消导入、导出账号（2026-09-23）

- 后端在用户关掉文件对话框时返回错误文本 `导入已取消` / `导出已取消`，前端对这两种情况不提示。
- 判断用 `utils/errors.js` 的 `getRawErrorMessage` 取后端原文比较。`getErrorMessage` 会先翻译成界面语言，英文界面拿到的是 `Import canceled`，旧代码因此把取消当成失败弹出提示。
- 验证：
  - 离线：`scripts/test-error-messages.mjs`（各种错误形状都取到原文；英文下原文会被翻译，说明必须比原文）。
  - 渲染检查（替身后端）：英文界面点“导出账号”和“导入 JSON 备份”，替身返回取消。修改前弹出 `Export canceled`，修改后两处都不弹提示。
  - TODO(verify)：真实文件对话框里取消还没试过（隔离运行不调导入导出，因为会弹对话框）。触发条件：下次在开发版手动测试账号导入导出时。通过判据：英文界面取消后不出现提示；选一个无效文件时仍提示失败原因。
