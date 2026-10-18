import { useI18n } from '../../i18n';
import { INTELLIGENCE_TESTS, intelligenceSummary, svgImageSource } from '../../utils/apiIntelligence';
import { formatExactTokens } from '../../utils/usageStatsFormat';
import { formatSeconds } from '../../utils/usageBills';

const STATUS_LABELS = {
  loading: '测试中',
  pass: '通过',
  fail: '未通过',
  visual: '请目测',
  empty: '未作答',
  error: '出错'
};

function IntelligenceRow({ result, test }) {
  const { language, t, translateRuntimeText } = useI18n();
  const status = !result || result.loading ? 'loading' : result.grade.status;
  const grade = result && result.grade;
  const response = result && result.response;
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
    <section className={`api-iq-row ${status}`}>
      <div className="api-iq-row-head">
        <span className="api-iq-row-title">{t(test.title)}</span>
        <span className={`api-iq-status ${status}`}>{t(STATUS_LABELS[status])}</span>
      </div>
      {status === 'visual' && (
        <img className="api-iq-svg" src={svgImageSource(grade.svg)} alt={t('鹈鹕变形测试生成的图')} />
      )}
      {(status === 'pass' || status === 'fail') && grade.answer && (
        <div className="api-iq-line">
          <em>{t('回答')}</em>
          <span>{grade.answer}</span>
        </div>
      )}
      {test.expected && status !== 'loading' && (
        <div className="api-iq-line">
          <em>{t('标准答案')}</em>
          <span>{t(test.expected)}</span>
        </div>
      )}
      {status === 'empty' && (
        <div className="api-iq-line">
          {grade.answer === 'max_output_tokens' ? t('达到输出上限仍未作答') : t('模型没有返回文字')}
        </div>
      )}
      {status === 'error' && <div className="api-iq-line api-iq-error">{translateRuntimeText(grade.answer)}</div>}
      {meta && <div className="api-iq-meta">{meta}</div>}
    </section>
  );
}

export default function ApiIntelligencePanel({ model, run }) {
  const { t } = useI18n();
  if (!run) {
    return (
      <div className="api-check-placeholder api-iq-placeholder">
        {t('用上方的测试模型同时发送 4 道题：糖果题、鹈鹕变形和两道不联网的知识题。鹈鹕图需要你目测，推理题可能要几分钟。')}
      </div>
    );
  }
  const summary = intelligenceSummary(run.results);
  return (
    <div className="api-iq-panel">
      <div className="api-iq-summary">
        <span>
          {summary.done
            ? t('自动判定通过 {passed}/{total}', { passed: summary.passed, total: summary.total })
            : t('测试中…')}
        </span>
        {run.model !== model && <span className="api-iq-model-note">{t('结果来自 {model}', { model: run.model })}</span>}
      </div>
      {INTELLIGENCE_TESTS.map(test => (
        <IntelligenceRow key={test.id} result={run.results[test.id]} test={test} />
      ))}
    </div>
  );
}
