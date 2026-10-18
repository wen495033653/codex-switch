import { useI18n } from '../i18n';
import { INTELLIGENCE_TESTS, intelligenceSummary, svgImageSource } from '../utils/intelligenceTests';
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
      {grade.status === 'visual' && (
        <img className="iq-svg" src={svgImageSource(grade.svg)} alt={t('鹈鹕变形测试生成的图')} />
      )}
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
        {run && run.model !== model && <span className="iq-model-note">{t('结果来自 {model}', { model: run.model })}</span>}
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
