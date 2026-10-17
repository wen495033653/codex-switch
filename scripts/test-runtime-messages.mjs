import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

let server;
let translate;
let translateRuntimeText;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ translate, translateRuntimeText } = await server.ssrLoadModule('/src/i18n.jsx'));
});

after(async () => { await server?.close(); });

// Texts as the backend sends them in command results, status events and auto-disable notices.
const BACKEND_MESSAGES = [
  ['设置已保存', 'Settings saved'],
  ['gpt破限已启用，重启 Codex 后生效。', 'GPT unlock enabled. Restart Codex to apply.'],
  ['gpt破限已关闭，重启 Codex 后生效。', 'GPT unlock disabled. Restart Codex to apply.'],
  ['远程控制账号已更新，重启 Codex 后生效', 'The remote control account was updated and will take effect after Codex restarts.'],
  ['控制账号登录已失效，已关闭远程控制', 'The control account sign-in is no longer valid, so remote control was disabled'],
  ['远程控制账号不存在，已关闭远程控制并切换到 API 模式', 'The remote control account no longer exists, so remote control was disabled and Codex switched to API mode'],
  ['已删除；远程控制账号已重置并切换到 API 模式，重启 Codex 后生效', 'Deleted. The remote control account was reset and Codex switched to API mode; restart Codex to apply.'],
  ['已切换到 API 模式；重新打开 IDE 前会同步会话', 'Switched to API mode; sessions are synced before the IDE reopens'],
  ['账号已添加，正在同步配额', 'Account added; syncing quotas'],
  ['回调 URL 已提交，正在完成 OAuth 登录', 'Callback URL submitted; completing OAuth sign-in'],
  ['账号信息已刷新（订阅与配额已更新）', 'Account refreshed (subscription and quotas updated)'],
  ['后台刷新仍在进行中', 'A background refresh is still running'],
  ['后台刷新中（3/10）', 'Refreshing in background (3/10)'],
  ['已刷新 9 个账号', '9 accounts refreshed'],
  ['已刷新 9 个账号，1 个失败', '9 accounts refreshed, 1 failed'],
  ['配额刷新失败\nUsage refresh failed', 'Quota refresh failed\nUsage refresh failed'],
  ['已用订阅账号 fixture-...7890打开 Codex', 'Opened Codex with subscription account fixture-...7890'],
  ['已用API Fixture API打开 Codex', 'Opened Codex with API Fixture API'],
  ['导入成功 2 个账号，导入失败 1 个，配额刷新失败 1 个\n失败原因：fixture error（1 个）', '2 accounts imported, 1 failed to import, quota refresh failed for 1\nFailure reasons: fixture error（1 个）'],
  ['导出成功（3 个 refresh_token）', 'Export complete (3 refresh_tokens)'],
  ['Codex 或 VS Code 已重新打开', 'Codex or VS Code reopened'],
  ['未能重新打开 Codex、VS Code x2', 'Could not reopen Codex, VS Code x2'],
  ['Codex 已重新打开；会话同步失败：fixture sync error', 'Codex reopened; session sync failed: fixture sync error'],
  ['未能重新打开 Codex', 'Could not reopen Codex'],
  ['模型列表认证失败 HTTP 401', 'Model list authentication failed: HTTP 401'],
  ['Responses 测试返回 HTTP 502', 'Responses test returned HTTP 502'],
  ['导入完成：2 个导入，0 个跳过，1 个冲突，1 个失败', '2 imported, 0 skipped, 1 conflicts, 1 failed'],
  ['已删除 2 个会话，1 个失败：a.jsonl: fixture', '2 sessions deleted, 1 failed: a.jsonl: fixture'],
  ['已恢复 2 个会话；fixture warning', '2 sessions restored; fixture warning'],
  ['已切换 3 个会话状态，1 个失败：b.jsonl: fixture', '3 session statuses updated, 1 failed: b.jsonl: fixture']
];

test('backend result messages have an English translation', () => {
  for (const [source, english] of BACKEND_MESSAGES) {
    assert.equal(translateRuntimeText(source, 'en'), english, source);
  }
});

test('backend result messages stay as sent in the Chinese interface', () => {
  for (const [source] of BACKEND_MESSAGES) {
    assert.equal(translateRuntimeText(source, 'zh-CN'), source);
  }
});

test('messages the frontend composes around a backend message are translated as a whole', () => {
  const backendMessage = translateRuntimeText('控制账号登录已失效，已关闭远程控制', 'en');
  assert.equal(
    translate('{message}；重启 Codex 后生效', { message: backendMessage }, 'en'),
    'The control account sign-in is no longer valid, so remote control was disabled; restart Codex to apply'
  );
  assert.equal(
    translate('{message}；无法确认 Codex 运行状态，建议重启', { message: backendMessage }, 'en'),
    'The control account sign-in is no longer valid, so remote control was disabled; the Codex process state could not be confirmed, so a restart is recommended'
  );
  assert.equal(
    translate('{message}\n本地文件已备份：{path}', { message: translateRuntimeText('gpt破限已启用，重启 Codex 后生效。', 'en'), path: 'fixture.bak' }, 'en'),
    'GPT unlock enabled. Restart Codex to apply.\nLocal file backed up: fixture.bak'
  );
  assert.equal(
    translate('{message} 远程控制代理更新失败：{error}', { message: translateRuntimeText('Codex 代理配置已启用，重启 Codex 后生效。', 'en'), error: 'fixture error' }, 'en'),
    'Proxy configuration enabled. Restart Codex to apply. Updating the remote control proxy failed: fixture error'
  );
  for (const text of ['远程控制已关闭并清空账号；API 运行配置同步失败，请重试', '远程控制已关闭并切换到 API 模式', '控制账号已删除，远程控制已关闭；重启 Codex 后生效。']) {
    assert.notEqual(translateRuntimeText(text, 'en'), text, text);
  }
  assert.equal(translate('{message}；重启 Codex 后生效', { message: '远程控制已关闭' }, 'zh-CN'), '远程控制已关闭；重启 Codex 后生效');
});
