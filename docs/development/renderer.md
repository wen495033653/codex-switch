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

## 卡片网格的分页（2026-09-23）

- 账号页和 API 页每页显示的卡片数，等于网格里能放下的列数乘整行数，由 `hooks/useGridPageSize.js` 测量；量不到卡片高度前按窗口高度取 4、6、9。
- 网格用 callback ref 交给这个 hook。网格节点每次挂载都会重新测量，`ResizeObserver` 也跟着换到新节点。原因：账号页的分页 hook 常驻在 App 里，网格却随页面切换卸载重建；旧代码只在第一次挂载时绑定，之后在别的页面改了窗口大小再回来，每页数量停在旧值。
- 页脚是 `components/Pagination.jsx`。只有一页时，账号页仍显示页码按钮，API 页不显示（`hideSinglePage`）。API 页在 `d1488e3` 有意从 `> 0` 改成 `> 1`，账号页从没改过，无法确认两边是否应该一致，所以各自保留。
- 验证：
  - 离线：`scripts/test-grid-pagination.mjs`（列数、行数的计算；两种页脚条件；页脚标记与改动前两页输出的标记逐字相同）。
  - 渲染检查（替身后端，40 个账号、30 个 API 配置）：修改前，1024×768 下每页 3 个，切到 API 页把窗口改成 1400×1000 再回账号页，能放 6 个却仍显示 3 个；修改后同样的步骤显示 6 个。回到账号页后只改网格高度（不改窗口），每页数量随之变化，说明新节点被观察。改动前后两页的主内容区 DOM 和页脚 DOM 逐字相同（1 个账号 / 1 个配置时也相同）。
  - TODO(verify)：真实窗口里拖动大小还没试过。触发条件：下次在开发版手动测试界面时。做法：在 API 页或会话页拖动窗口高度后回到账号页。通过判据：最后一行卡片完整显示，没有空出整行也没有被裁掉。

## 设置加载前的界面语言（2026-09-23）

- 主窗口在设置加载完成前，用 `localStorage` 里上次保存的语言（`getStoredLanguagePreference`），并且不写回；加载完成后按设置里的语言显示并写入 `localStorage`。开发日志窗口照旧读取这份本地值。
- 原因：旧代码加载前用 `DEFAULT_SETTINGS.ui_language`（`zh-CN`），并立即写进 `localStorage`。英文用户每次启动先闪一下中文；设置加载失败时本地值被改成 `zh-CN`，之后一直是中文。
- 主题没有改：它没有本地缓存，加载前用系统主题（`main.jsx` 和默认值 `system` 一致），也不写任何存储。用户选的主题和系统不同时，启动瞬间仍会先显示系统主题。
- 验证：
  - 渲染检查（替身后端）：本地存 `en`、设置加载失败。修改前界面是中文、本地值被改成 `zh-CN`；修改后界面是英文、本地值仍是 `en`。本地存 `en`、设置为 `zh-CN` 且延迟 6 秒返回：加载前英文且本地值不变，加载后中文且本地值变为 `zh-CN`。
  - TODO(verify)：真实应用启动还没看过。触发条件：下次按 [dev-preview.md](dev-preview.md) 做隔离环境真实运行时。通过判据：界面语言设为 English 后重启，首屏没有中文，`localStorage` 的 `codex-switch.ui-language` 保持 `en`。

## 失败不再静默（2026-09-23）

- 用户点出来的操作失败时弹提示：更新弹窗点“稍后”时记录跳过版本失败（`dismiss_update_version`）、重新打开提示点“稍后”时丢弃快照失败（`discard_ide_snapshot`）、进入设置页时读取设置失败（仍然进入设置页，显示上次的设置）、Codex 页“更新 ChatGPT”打开网页失败。
- 后台请求失败写 `console.error`，开发版会进入开发日志窗口。启动时的自动检查更新失败记一条。每隔几秒的轮询（`get_codex_app_instance_status`、`usage_stats_get`）用 `utils/pollingErrorLog.js`：只记第一次失败、错误内容变化和恢复，都带连续失败次数，避免每次轮询一条挤掉开发日志（上限 160 条）里的其它内容。`usage_stats_get` 返回 `ok` 不为 `true` 时也按失败记录原始返回。
- 事件监听注册失败时记一条错误，说明这个窗口收不到该事件；取消监听失败也记录。
- “更新 ChatGPT”的打开逻辑从视图组件移到 `useSettingsActions`，由 `CodexPage` 通过 props 传入。
- 验证：
  - 离线：`scripts/test-background-errors.mjs`（事件注册失败只记一条、取消监听失败有记录；轮询失败按“首次、变化、恢复”记录）。
  - 渲染检查（替身后端，替身把页面报成可见）：四个用户操作的失败都弹出后端原文；多开状态轮询失败 4 次只记 1 条 `[get_codex_app_instance_status] background request failed (consecutive failures: 1)`；token 统计和自动检查更新失败各记 1 条。
  - TODO(verify)：真实应用里没有触发过这些失败。触发条件：下次按 [dev-preview.md](dev-preview.md) 做隔离环境真实运行时，检查脚本里对 `get_codex_app_instance_status` 返回一次错误。通过判据：开发日志窗口出现带 `consecutive failures` 的记录，恢复后出现 `recovered after`。

## 独立 Codex 运行状态的轮询（2026-09-23）

- 只有账号页和 API 页的卡片显示“窗口运行中”，所以 `get_codex_app_instance_status` 只在这两个页面每 3 秒轮询一次；进入这两个页面时立即查一次，窗口获得焦点时也查一次。
- 状态只保留 `runningByKey`（卡片读取的唯一字段），去掉了没有任何地方读取的 `instances`、`instancesByKey`、`runningByTargetKey`。每次轮询先比较，运行中的实例没有变化就沿用原对象，不触发 App 重新渲染。
- 原因：旧代码在所有页面都轮询，并且每次都用新对象更新 App 的状态，整棵组件树每 3 秒重渲一次。
- 验证：
  - 离线：`scripts/test-codex-app-instances.mjs`（只保留运行中的实例；结果不变时比较相等）。
  - 渲染检查（替身后端，页面报成可见，各页面停留 9 秒，React 提交次数用 DevTools hook 计数）：修改前每个页面都请求 3 次，账号、API、设置页各提交 3 次，Codex 页 6 次（另 3 次来自进程卡片自己的轮询）；修改后会话、设置、Codex 页不再请求，账号页请求 3 次、提交 0 次，Codex 页只剩进程卡片的提交。替身返回一个运行中的实例时，对应账号卡片显示“窗口运行中”，从设置页回到账号页时立即请求一次。
  - TODO(verify)：真实应用里没有看过。触发条件：下次在开发版里开一个独立 Codex 窗口时。通过判据：账号页和 API 页上的“窗口运行中”在窗口打开、关闭后 3 秒内更新；停留在会话页时开发日志里没有 `get_codex_app_instance_status` 请求。
