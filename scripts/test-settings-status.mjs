import assert from 'node:assert/strict';
import test from 'node:test';
import { proxySaveFeedback } from '../renderer/src/utils/codexSettingsStatus.js';
test('proxy save preserves restart requirement and surfaces runtime failure', () => {
  const result = proxySaveFeedback({ message: '配置已保存', restartRequired: true, remoteControl: { error: 'fixture connection error' } });
  assert.equal(result.restartRequired, true);
  assert.equal(result.warning, true);
  assert.match(result.message, /fixture connection error/);
  assert.equal(proxySaveFeedback({ message: '配置已保存' }).warning, false);
});
