import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const src = new URL('../renderer/src/', import.meta.url);
const a = { id: 'a', name: 'A', base_url: 'https://a.example/v1', api_key: 'fixture-a' };
const b = { id: 'b', name: 'B', base_url: 'https://b.example/v1', api_key: 'fixture-b' };

async function fixture(api) {
  let cursor = 0;
  const values = [];
  const calls = [];
  const context = vm.createContext({ URL, window: { api } });
  const stubs = {
    react: { useState(initial) {
      const index = cursor++;
      if (!(index in values)) values[index] = typeof initial === 'function' ? initial() : initial;
      return [values[index], value => {
        values[index] = typeof value === 'function' ? value(values[index]) : value;
      }];
    } },
    '../utils/auth': { API_MODE_ACCOUNT_ID: '__api__' },
    '../utils/errors': { getErrorMessage: error => error.message }
  };
  async function hook(name) {
    const url = new URL('hooks/' + name + '.js', src);
    const module = new vm.SourceTextModule(readFileSync(url, 'utf8'), { context });
    await module.link(specifier => {
      const exports = stubs[specifier];
      if (exports) return new vm.SyntheticModule(Object.keys(exports), function () {
        for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
      }, { context });
      return new vm.SourceTextModule(readFileSync(new URL(specifier + '.js', url), 'utf8'), { context });
    });
    await module.evaluate();
    return module.namespace[name];
  }
  const log = name => value => calls.push([name, value]);
  return {
    calls, hook,
    render(fn, overrides = {}) {
      cursor = 0;
      return fn({
        apiModeActive: true,
        activeApiProfileId: 'a',
        applySettings: log('settings'),
        handleRes: log('store'),
        onUsageStatsRefresh: () => calls.push(['refresh']),
        showIdeReopen: log('reopen'),
        toastError: err => calls.push(['error', err.message]),
        ...overrides
      });
    }
  };
}

test('API switching sends one authoritative operation and publishes state only when it succeeds', async () => {
  let reject = false;
  const result = { settings: { active_api_profile_id: 'b' }, ide_reopen: { snapshot_id: 'fixture' } };
  const f = await fixture({
    async switchApiMode(id) {
      f.calls.push(['switch', id]);
      if (reject) throw new Error('fixture switch failed');
      return result;
    }
  });
  const switchMode = await f.hook('useModeSwitching');
  await f.render(switchMode).switchToApiModeFromPage('b');
  assert.deepEqual(f.calls, [
    ['switch', 'b'], ['settings', result], ['store', result], ['refresh'], ['reopen', result.ide_reopen]
  ]);

  f.calls.length = 0;
  reject = true;
  await f.render(switchMode).switchToApiModeFromPage('b');
  assert.deepEqual(f.calls, [['switch', 'b'], ['error', 'fixture switch failed']]);
});

test('profile editing and deletion send only the list and never switch the active API silently', async () => {
  const f = await fixture({
    async updateSettings(patch) {
      f.calls.push(['save', JSON.parse(JSON.stringify(patch))]);
      return { ok: true };
    }
  });
  const draft = await f.hook('useApiModeDraft');
  const render = options => f.render(draft, options);
  render().setApiProfiles([a, b]);
  render().setActiveApiProfileId(a.id);

  render().openDeleteApiProfileModal('a');
  assert.equal(render().apiProfileDeleteModal.visible, false);
  assert.match(f.calls[0][1], /先切换/);

  render().openDeleteApiProfileModal('b');
  await render().confirmDeleteApiProfile();
  assert.deepEqual(f.calls[1], ['save', { api_profiles: [a] }]);
  render({ apiModeActive: false }).openDeleteApiProfileModal('a');
  await render({ apiModeActive: false }).confirmDeleteApiProfile();
  assert.deepEqual(f.calls[3], ['save', { api_profiles: [b] }]);

  render().editApiProfile('b');
  render().updateApiProfileModalDraft({ name: 'B edited' });
  await render().saveApiProfileModal();
  assert.deepEqual(f.calls[5], ['save', { api_profiles: [a, { ...b, name: 'B edited' }] }]);
});
