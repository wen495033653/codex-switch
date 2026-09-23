import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { getGridPageMetrics } from '../renderer/src/utils/appState.js';

let server;
let Pagination;
let I18nProvider;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ default: Pagination } = await server.ssrLoadModule('/src/components/Pagination.jsx'));
  ({ I18nProvider } = await server.ssrLoadModule('/src/i18n.jsx'));
});

after(async () => { await server?.close(); });

test('grid page metrics count the columns and the whole rows that fit', () => {
  const grid = { templateColumns: '304px 304px 304px', rowGap: 14, cardHeight: 264 };
  assert.deepEqual(getGridPageMetrics({ ...grid, clientHeight: 517 }), { columns: 3, rows: 1 });
  assert.deepEqual(getGridPageMetrics({ ...grid, clientHeight: 749 }), { columns: 3, rows: 2 });
  assert.deepEqual(getGridPageMetrics({ ...grid, clientHeight: 0 }), { columns: 3, rows: 1 });
  assert.deepEqual(getGridPageMetrics({ ...grid, templateColumns: '' , clientHeight: 542 }), { columns: 1, rows: 2 });
  assert.equal(getGridPageMetrics({ ...grid, cardHeight: 0, clientHeight: 517 }), null);
});

function renderFooter(props, language = 'zh-CN') {
  return renderToStaticMarkup(createElement(I18nProvider, { preference: language },
    createElement(Pagination, { onPageChange() {}, page: 1, pageSize: 3, startIdx: 0, ...props })));
}

test('accounts keep the page buttons on a single page, the API page hides them', () => {
  const single = { total: 1, totalPages: 1 };
  assert.match(renderFooter(single), /class="pagination"/);
  assert.doesNotMatch(renderFooter({ ...single, hideSinglePage: true }), /class="pagination"/);
  assert.doesNotMatch(renderFooter({ total: 0, totalPages: 0 }), /class="pagination"/);
  assert.match(renderFooter({ total: 7, totalPages: 3, hideSinglePage: true }), /class="pagination"/);
});

test('the footer markup matches the markup both pages rendered before', () => {
  assert.equal(
    renderFooter({ total: 1, totalPages: 1 }, 'en'),
    '<div class="panel-footer"><div class="footer-info">Showing 1–1 of 1</div><div class="pagination">'
      + '<button type="button" class="page-btn" aria-label="Previous" disabled="">&lt;</button>'
      + '<button type="button" class="page-btn active" aria-current="page">1</button>'
      + '<button type="button" class="page-btn" aria-label="Next" disabled="">&gt;</button></div></div>'
  );
  assert.equal(
    renderFooter({ total: 1, totalPages: 1, hideSinglePage: true }, 'en'),
    '<div class="panel-footer"><div class="footer-info">Showing 1–1 of 1</div></div>'
  );
});
