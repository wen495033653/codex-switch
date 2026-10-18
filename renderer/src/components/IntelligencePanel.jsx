import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useI18n } from '../i18n';
import {
  INTELLIGENCE_TESTS,
  INTELLIGENCE_TESTS_DATE,
  intelligenceSummary,
  svgImageSource
} from '../utils/intelligenceTests';
import { formatExactTokens } from '../utils/usageStatsFormat';
import { formatSeconds } from '../utils/usageBills';

const STATUS_LABELS = {
  loading: '测试中',
  pass: '通过',
  fail: '未通过',
  visual: '请目测',
  empty: '未作答',
  error: '出错'
};

// Click to see it full size; click anywhere or press Esc to close.
function SvgPreview({ svg }) {
  const { t } = useI18n();
  const [zoomed, setZoomed] = useState(false);
  const source = svgImageSource(svg);
  const alt = t('鹈鹕变形测试生成的图');

  useEffect(() => {
    if (!zoomed) return undefined;
    const closeOnEscape = event => {
      if (event.key === 'Escape') setZoomed(false);
    };
    window.addEventListener('keydown', closeOnEscape);
    return () => window.removeEventListener('keydown', closeOnEscape);
  }, [zoomed]);

  return (
    <>
      <button type="button" className="iq-svg-button" title={t('点击放大')} onClick={() => setZoomed(true)}>
        <img className="iq-svg" src={source} alt={alt} />
      </button>
      {zoomed && createPortal(
        <div className="iq-svg-zoom" role="dialog" aria-modal="true" aria-label={alt} onClick={() => setZoomed(false)}>
          <img src={source} alt={alt} />
        </div>,
        document.body
      )}
    </>
  );
}

function IntelligenceResult({ result }) {
  const { language, t, translateRuntimeText } = useI18n();
  const { grade, response } = result;
  const usage = response && response.usage;
  const meta = response && response.ok
    ? [
      t('用时 {duration}', { duration: formatSeconds(response.durationMs) }),
      usage && usage.outputTokens !== null && usage.outputTokens !== undefined
        ? t('输出 {output} token（推理 {reasoning}）', {
          output: formatExactTokens(usage.outputTokens, language),
          reasoning: formatExactTokens(usage.reasoningTokens || 0, language)
        })
        : ''
    ].filter(Boolean).join(' · ')
    : '';

  return (
    <>
      {grade.status === 'visual' && <SvgPreview svg={grade.svg} />}
      {(grade.status === 'pass' || grade.status === 'fail') && grade.answer && (
        <div className="iq-line">
          <em>{t('回答')}</em>
          <span>{grade.answer}</span>
        </div>
      )}
      {grade.status === 'empty' && (
        <div className="iq-line">
          {grade.answer === 'max_output_tokens' ? t('达到输出上限仍未作答') : t('模型没有返回文字')}
        </div>
      )}
      {grade.status === 'error' && (
        <div className="iq-line iq-error">
          <span>{translateRuntimeText(grade.answer)}</span>
        </div>
      )}
      {grade.status === 'error' && grade.detail && <pre className="iq-error-detail">{grade.detail}</pre>}
      {meta && <div className="iq-meta">{meta}</div>}
    </>
  );
}

function IntelligenceRow({ checked, disabled, onToggle, result, test }) {
  const { t } = useI18n();
  const status = result ? (result.loading ? 'loading' : result.grade.status) : '';
  return (
    <section className={`iq-row ${checked ? '' : 'unchecked'}`}>
      <div className="iq-row-head">
        <label className="iq-row-label">
          <input type="checkbox" checked={checked} disabled={disabled} onChange={() => onToggle(test.id)} />
          <span className="iq-row-title">{t(test.title)}</span>
        </label>
        <span className="iq-tag">{t(test.category)}</span>
        <span className="iq-tag">{test.expected ? t('自动判分') : t('需目测')}</span>
        {status && <span className={`iq-status ${status}`}>{t(STATUS_LABELS[status])}</span>}
      </div>
      <div className="iq-row-body">
        <div className="iq-row-summary">{t(test.summary)}</div>
        <details className="iq-prompt">
          <summary>{t('题目原文')}</summary>
          <pre>{test.prompt}</pre>
        </details>
        {test.expected && (
          <div className="iq-line">
            <em>{t('标准答案')}</em>
            <span>{t(test.expected)}</span>
          </div>
        )}
        {result && !result.loading && <IntelligenceResult result={result} />}
      </div>
    </section>
  );
}

function summaryText(run, t) {
  if (!run) return t('勾选的题会同时发出，推理题可能要几分钟');
  const summary = intelligenceSummary(run.results);
  if (!summary.done) {
    return t('测试中…（{finished}/{tested}）', { finished: summary.finished, tested: summary.tested });
  }
  return summary.graded > 0
    ? t('自动判定通过 {passed}/{total}', { passed: summary.passed, total: summary.graded })
    : t('测试完成');
}

// The question list of a 降智 test: tick the questions to send; each one's result shows in its row.
export default function IntelligencePanel({ model, onToggleTest, run, running, selectedTestIds }) {
  const { t } = useI18n();
  return (
    <div className="iq-panel">
      <div className="iq-summary">
        <span>{summaryText(run, t)}</span>
        <span className="iq-summary-side">
          {run && run.model !== model && <span className="iq-model-note">{t('结果来自 {model}', { model: run.model })}</span>}
          <span className="iq-bank-date">{t('题库更新于 {date}', { date: INTELLIGENCE_TESTS_DATE })}</span>
        </span>
      </div>
      {INTELLIGENCE_TESTS.map(test => (
        <IntelligenceRow
          key={test.id}
          checked={selectedTestIds.includes(test.id)}
          disabled={running}
          onToggle={onToggleTest}
          result={run && run.results[test.id]}
          test={test}
        />
      ))}
    </div>
  );
}

export function IntelligenceStartButton({ disabled, onStart, run, running, selectedCount }) {
  const { t } = useI18n();
  let label = t('开始测试（{count} 题）', { count: selectedCount });
  if (running) {
    label = t('测试中…');
  } else if (run) {
    label = t('重新测试（{count} 题）', { count: selectedCount });
  }
  return (
    <button
      type="button"
      className="btn btn-primary iq-start-button"
      disabled={disabled || running || selectedCount === 0}
      onClick={onStart}
    >
      {label}
    </button>
  );
}

// Shown on the card while its test runs, so a closed dialog still tells that answers are coming.
export function IntelligenceRunningTag({ progress }) {
  const { t } = useI18n();
  const counts = { finished: progress.finished, tested: progress.tested };
  return (
    <span className="iq-running-tag" title={t('降智测试进行中，已完成 {finished}/{tested} 题', counts)}>
      <span className="iq-running-spinner" aria-hidden="true" />
      <span>{t('降智测试 {finished}/{tested}', counts)}</span>
    </span>
  );
}
