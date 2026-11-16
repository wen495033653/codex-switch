import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import { transformWithOxc } from 'vite';

const src = new URL('../renderer/src/', import.meta.url);
const root = '/fixture/codex';
const active = { title: 'Active fixture', id: 'official-active-id', status: 'active', relative_path: 'sessions/active.jsonl' };
const archived = { title: 'Archived fixture', id: 'official-archived-id', status: 'archived', relative_path: 'archived_sessions/archived.jsonl' };

async function fixture({ failedId } = {}) {
  const values = [], calls = [], toasts = [], errors = [];
  let cursor = 0;
  let conversations = [active, archived];
  const preview = Promise.withResolvers();
  const window = {};
  const jsx = (type, props) => ({ type, props });
  const context = vm.createContext({ window, requestAnimationFrame: callback => callback() });
  const stubs = {
    react: {
      useState(initial) {
        const index = cursor++;
        if (!(index in values)) values[index] = typeof initial === 'function' ? initial() : initial;
        return [values[index], value => { values[index] = typeof value === 'function' ? value(values[index]) : value; }];
      },
      useRef(initial) {
        const index = cursor++;
        return values[index] ??= { current: initial };
      },
      useMemo: compute => compute(),
      useEffect() {},
      useId: () => 'fixture-id'
    },
    'react/jsx-runtime': { jsx, jsxs: jsx, Fragment: Symbol('Fragment') },
    i18n: { useI18n: () => ({
      language: 'zh-CN',
      t: (key, params = {}) => key.replace(/\{(\w+)\}/g, (_, name) => params[name]),
      translateRuntimeText: text => text
    }) },
    '@tauri-apps/api/core': {
      isTauri: () => true,
      async invoke(command, args) {
        calls.push([command, JSON.parse(JSON.stringify(args))]);
        if (command === 'session_manager_scan') return { root, conversations };
        if (command === 'session_manager_preview') return preview.promise;
        assert.equal(command, 'session_manager_delete');
        const previousCount = conversations.length;
        conversations = conversations.filter(item => !args.threadIds.includes(item.id) || item.id === failedId);
        const deleted = previousCount - conversations.length;
        const failed = args.threadIds.includes(failedId) ? 1 : 0;
        const error = `${failedId}: fixture permission error`;
        return {
          ok: failed === 0,
          message: `已永久删除 ${deleted} 个会话${failed ? `，1 个失败：${error}` : ''}`,
          report: { deleted, failed, errors: failed ? [error] : [] }
        };
      }
    },
    '@tauri-apps/api/event': { listen() {} }
  };
  const modules = new Map();
  function load(id) {
    if (!modules.has(id)) modules.set(id, (async () => {
      const exports = stubs[id];
      if (exports) return new vm.SyntheticModule(Object.keys(exports), function () {
        for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
      }, { context, identifier: id });
      const source = fs.readFileSync(new URL(id), 'utf8');
      const code = id.endsWith('.jsx')
        ? (await transformWithOxc(source, id, { jsx: { runtime: 'automatic' } })).code
        : source;
      return new vm.SourceTextModule(code, { context, identifier: id });
    })());
    return modules.get(id);
  }
  async function evaluate(path) {
    const module = await load(new URL(path, src).href);
    await module.link((specifier, parent) => {
      if (specifier.endsWith('/i18n')) return load('i18n');
      if (stubs[specifier]) return load(specifier);
      const js = new URL(`${specifier}.js`, parent.identifier);
      return load(fs.existsSync(js) ? js.href : new URL(`${specifier}.jsx`, parent.identifier).href);
    });
    await module.evaluate();
    return module.namespace;
  }

  const { default: Page } = await evaluate('components/SessionManagerPage.jsx');
  const { useSessionManagerState } = await evaluate('hooks/useSessionManagerState.js');
  const { installTauriApiBridge } = await evaluate('desktopApi.js');
  installTauriApiBridge();
  const f = {
    api: window.api, calls, toasts, errors, preview,
    render() {
      cursor = 0;
      return nodes(Page({
        sessionState: useSessionManagerState(),
        toast: message => toasts.push(message),
        toastError: (error, label) => errors.push([error.message, label])
      }));
    }
  };
  await button(f.render(), '刷新').props.onClick();
  return f;
}

function nodes(value) {
  if (Array.isArray(value)) return value.flatMap(nodes);
  if (!value?.props) return [];
  if (typeof value.type === 'function') return nodes(value.type(value.props));
  return [value, ...nodes(value.props.children)];
}

function button(tree, label, role) {
  const item = tree.find(node => node.type === 'button' && node.props.children === label && (!role || node.props.role === role));
  assert.ok(item, `Missing ${label} button`);
  return item;
}

test('context menu permanently deletes only after confirmation and drops the stale preview', async () => {
  const f = await fixture();
  for (const command of ['previewDeletedSession', 'listDeletedSessions', 'restoreDeletedSessions', 'purgeDeletedSessions']) {
    assert.equal(command in f.api, false);
  }
  assert.deepEqual(f.render().filter(node => node.props.className === 'session-status-label').map(node => node.props.children),
    ['全部', '未归档', '已归档']);

  const row = f.render().find(node => node.props['aria-label'] === archived.title);
  const showConfirmation = () => {
    row.props.onContextMenu({ preventDefault() {}, stopPropagation() {} });
    button(f.render(), '永久删除', 'menuitem').props.onClick();
    const dialog = f.render();
    assert.ok(dialog.some(node => node.props.children === '将永久删除所选 1 个会话及其子代理会话，删除后不可恢复。'));
    return dialog;
  };
  button(showConfirmation(), '取消').props.onClick();
  assert.equal(f.render().some(node => node.props.role === 'dialog'), false);
  assert.deepEqual(f.calls.map(([command]) => command), ['session_manager_scan']);

  const previewRequest = row.props.onClick();
  await button(showConfirmation(), '永久删除').props.onClick();
  f.preview.resolve({ conversation: archived, messages: [] });
  await previewRequest;

  assert.deepEqual(f.calls.slice(2), [
    ['session_manager_delete', { root, threadIds: [archived.id] }],
    ['session_manager_scan', { root }]
  ]);
  assert.deepEqual(f.toasts, ['已永久删除 1 个会话']);
  assert.deepEqual(f.errors, []);
  const tree = f.render();
  assert.equal(tree.some(node => node.props['aria-label'] === archived.title || node.props.children === '撤销'), false);
  assert.ok(tree.some(node => node.props.children === '选择一条会话查看预览'));
});

test('bulk deletion presents the real partial error and keeps failed rows selected', async () => {
  const f = await fixture({ failedId: archived.id });
  f.render().find(node => node.props['aria-label'] === '选择本页会话').props.onChange();
  button(f.render(), '永久删除').props.onClick();
  const dialog = f.render();
  assert.ok(dialog.some(node => node.props.children === '将永久删除所选 2 个会话及其子代理会话，删除后不可恢复。'));
  await dialog.find(node => node.props.className?.includes('confirm-dialog-confirm')).props.onClick();

  assert.deepEqual(f.calls.slice(1), [
    ['session_manager_delete', { root, threadIds: [active.id, archived.id] }],
    ['session_manager_scan', { root }]
  ]);
  assert.deepEqual(f.errors, [[`已永久删除 1 个会话，1 个失败：${archived.id}: fixture permission error`, '删除会话失败']]);
  assert.deepEqual(f.toasts, []);
  const tree = f.render();
  assert.equal(tree.some(node => node.props['aria-label'] === active.title), false);
  assert.equal(tree.find(node => node.props['aria-label'] === `选择会话：${archived.title}`).props.checked, true);
});
