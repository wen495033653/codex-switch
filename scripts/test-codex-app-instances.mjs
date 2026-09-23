import assert from 'node:assert/strict';
import test from 'node:test';
import {
  isSameCodexAppInstanceStatus,
  markCodexAppInstanceRunning,
  normalizeCodexAppInstanceStatus
} from '../renderer/src/utils/codexAppInstances.js';

const response = running => ({
  instances: [
    { instanceKey: 'account-fixture-a', targetKey: 'account:fixture-a', running, pid: running ? 101 : null },
    { instanceKey: 'api-fixture-b', targetKey: 'api:fixture-b', running: false },
    { instanceKey: '', running: true }
  ]
});

test('the status keeps only the running instance keys', () => {
  assert.deepEqual(normalizeCodexAppInstanceStatus(response(true)), { runningByKey: { 'account-fixture-a': true } });
  assert.deepEqual(normalizeCodexAppInstanceStatus(null), { runningByKey: {} });
  assert.deepEqual(
    markCodexAppInstanceRunning(normalizeCodexAppInstanceStatus(null), { instanceKey: 'api-fixture-b' }),
    { runningByKey: { 'api-fixture-b': true } }
  );
  const status = normalizeCodexAppInstanceStatus(null);
  assert.equal(markCodexAppInstanceRunning(status, {}), status);
});

test('an unchanged poll result compares equal, so the previous state is kept', () => {
  const first = normalizeCodexAppInstanceStatus(response(true));
  assert.equal(isSameCodexAppInstanceStatus(first, normalizeCodexAppInstanceStatus(response(true))), true);
  assert.equal(isSameCodexAppInstanceStatus(first, normalizeCodexAppInstanceStatus(response(false))), false);
  assert.equal(
    isSameCodexAppInstanceStatus(first, markCodexAppInstanceRunning(first, { instanceKey: 'api-fixture-b' })),
    false
  );
  assert.equal(
    isSameCodexAppInstanceStatus({ runningByKey: { a: true } }, { runningByKey: { b: true } }),
    false
  );
});
