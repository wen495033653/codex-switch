import assert from 'node:assert/strict';
import test from 'node:test';
import { remoteControlPrerequisites, proxySaveFeedback } from '../renderer/src/utils/codexSettingsStatus.js';

test('remote control shows simultaneous mode and missing-account causes', () => {
  assert.deepEqual(remoteControlPrerequisites({ subscriptionMode: true, accountId: 'removed', accountPresent: false }),
    ['请先切换到 API 模式', '已选控制账号不存在，请重新选择']);
});
test('remote control distinguishes no selection, expired login and ready', () => {
  assert.deepEqual(remoteControlPrerequisites({ accountPresent: false }), ['请先选择远程控制账号']);
  assert.deepEqual(remoteControlPrerequisites({ accountPresent: true, authInvalid: true }), ['控制账号登录已过期，请重新登录']);
  assert.deepEqual(remoteControlPrerequisites({ accountPresent: true, authInvalid: false }), []);
});
test('proxy save preserves restart requirement and surfaces runtime failure', () => {
  const result = proxySaveFeedback({ message: '配置已保存', restartRequired: true, remoteControl: { error: 'fixture connection error' } });
  assert.equal(result.restartRequired, true);
  assert.equal(result.warning, true);
  assert.match(result.message, /fixture connection error/);
  assert.equal(proxySaveFeedback({ message: '配置已保存' }).warning, false);
});
