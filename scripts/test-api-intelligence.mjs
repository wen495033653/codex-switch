import assert from 'node:assert/strict';
import test from 'node:test';
import {
  INTELLIGENCE_TESTS,
  extractSvg,
  finalAnswerNumber,
  gradeIntelligenceResponse,
  intelligenceSummary,
  svgImageSource
} from '../renderer/src/utils/apiIntelligence.js';

const byId = Object.fromEntries(INTELLIGENCE_TESTS.map(item => [item.id, item]));
const answered = outputText => ({ ok: true, outputText });

test('the final answer is the number after the last 答案, or the last number', () => {
  assert.equal(finalAnswerNumber('先算 16 + 12 = 28，所以\n答案：29'), '29');
  assert.equal(finalAnswerNumber('答案: 28\n更正后答案：29'), '29');
  assert.equal(finalAnswerNumber('最少需要取出 29 个'), '29');
  assert.equal(finalAnswerNumber('无法确定'), null);
});

test('each question is graded against its expected answer', () => {
  assert.equal(gradeIntelligenceResponse(byId.candy, answered('……\n答案：29')).status, 'pass');
  assert.deepEqual(gradeIntelligenceResponse(byId.candy, answered('答案：21')), { status: 'fail', answer: '21' });
  assert.equal(
    gradeIntelligenceResponse(byId.thibault, answered('Thibault Sottiaux 是 OpenAI 的 Codex 负责人。')).status,
    'pass'
  );
  assert.equal(
    gradeIntelligenceResponse(byId.thibault, answered("I don't have reliable information about who he is.")).status,
    'fail'
  );
  assert.equal(gradeIntelligenceResponse(byId['japan-pm'], answered('高市早苗')).status, 'pass');
  assert.equal(gradeIntelligenceResponse(byId['japan-pm'], answered('石破茂')).status, 'fail');
});

test('the pelican is shown, not graded', () => {
  const svg = '<svg xmlns="http://www.w3.org/2000/svg"><circle r="4"/></svg>';
  const graded = gradeIntelligenceResponse(byId.pelican, answered(`\`\`\`svg\n${svg}\n\`\`\``));
  assert.deepEqual(graded, { status: 'visual', answer: '', svg });
  assert.equal(extractSvg('no image here'), '');
  assert.equal(gradeIntelligenceResponse(byId.pelican, answered('抱歉')).status, 'fail');
  assert.ok(svgImageSource(svg).startsWith('data:image/svg+xml;charset=utf-8,%3Csvg'));
});

test('failed requests and empty answers are told apart', () => {
  assert.deepEqual(
    gradeIntelligenceResponse(byId.candy, { ok: false, error: '测试请求返回 HTTP 502' }),
    { status: 'error', answer: '测试请求返回 HTTP 502' }
  );
  assert.deepEqual(
    gradeIntelligenceResponse(byId.candy, { ok: true, outputText: '', incompleteReason: 'max_output_tokens' }),
    { status: 'empty', answer: 'max_output_tokens' }
  );
});

test('the summary counts the graded questions only', () => {
  const results = {
    candy: { loading: false, grade: { status: 'pass' } },
    pelican: { loading: false, grade: { status: 'visual' } },
    thibault: { loading: false, grade: { status: 'fail' } },
    'japan-pm': { loading: true }
  };
  assert.deepEqual(intelligenceSummary(results), { passed: 1, total: 3, done: false });
  results['japan-pm'] = { loading: false, grade: { status: 'pass' } };
  assert.deepEqual(intelligenceSummary(results), { passed: 2, total: 3, done: true });
});
