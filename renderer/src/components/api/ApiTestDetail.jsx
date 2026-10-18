import { useI18n } from '../../i18n';
import { getApiCheckTimeText, getApiLastAvailableTimeText, getApiResponsesSummary, getApiTestState, getApiTestStateLabel, formatApiTestJson, getApiTestResponseBody, getApiTestResponseStatusLabel } from '../../utils/apiTestView';
import { formatExactTokens } from '../../utils/usageStatsFormat';

export function ApiTestResponseBlock({ response, title }) {
  const { t } = useI18n();
  if (!response) return null;
  const body = getApiTestResponseBody(response, t);

  return (
    <section className="api-test-response-block">
      <div className="api-test-response-head">
        <div className="api-test-response-title">{title}</div>
        <div className="api-test-response-status">{getApiTestResponseStatusLabel(response, t)}</div>
      </div>
      <div className="api-test-response-endpoint" title={response.endpoint || ''}>
        {response.endpoint || t('未返回 endpoint')}
      </div>
      <pre className="api-test-response-body">{body}</pre>
    </section>
  );
}

function ApiTestResponsesBlock({ request, response }) {
  const { t } = useI18n();
  if (!request && !response) return null;
  const endpoint = (response && response.endpoint) || (request && request.endpoint) || '';
  const requestBody = request ? formatApiTestJson(request.body || {}) : t('未发送请求');
  const responseBody = response ? getApiTestResponseBody(response, t) : t('未返回响应');

  return (
    <section className="api-test-response-block api-test-chat-block">
      <div className="api-test-response-head">
        <div className="api-test-response-title">{t('Responses 调用')}</div>
        <div className="api-test-response-status">{getApiTestResponseStatusLabel(response, t)}</div>
      </div>
      <div className="api-test-response-endpoint" title={endpoint}>
        {endpoint || t('未返回 endpoint')}
      </div>
      <div className="api-test-chat-grid">
        <div className="api-test-chat-pane">
          <div className="api-test-chat-pane-title">{t('请求')}</div>
          <pre className="api-test-response-body">{requestBody}</pre>
        </div>
        <div className="api-test-chat-pane">
          <div className="api-test-chat-pane-title">{t('返回')}</div>
          <pre className="api-test-response-body">{responseBody}</pre>
        </div>
      </div>
    </section>
  );
}

function ApiResponsesSummary({ response, summary, testModel }) {
  const { language, t } = useI18n();
  const modelDiffers = Boolean(summary.model && summary.model !== testModel);
  const items = [
    { key: 'status', label: t('HTTP 状态'), value: getApiTestResponseStatusLabel(response, t) },
    {
      key: 'model',
      label: t('返回模型'),
      value: summary.model || '—',
      warning: modelDiffers,
      title: modelDiffers ? t('返回的模型名和测试模型不同') : summary.model
    },
    { key: 'answer', label: t('回答'), value: summary.outputText || '—', title: summary.outputText },
    {
      key: 'tokens',
      label: 'Token',
      value: summary.totalTokens === null ? '—' : formatExactTokens(summary.totalTokens, language)
    }
  ];

  return (
    <div className="api-test-summary">
      {items.map(item => (
        <div key={item.key} className={`api-test-summary-item ${item.warning ? 'warning' : ''}`} title={item.title || undefined}>
          <em>{item.label}</em>
          <span>{item.value}</span>
        </div>
      ))}
    </div>
  );
}

export default function ApiTestDetailContent({ test }) {
  const { language, t, translateRuntimeText } = useI18n();
  const state = getApiTestState(test);
  const timeText = getApiCheckTimeText(test, t, language);
  const lastAvailableTimeText = getApiLastAvailableTimeText(test, t, language);
  const shouldShowMessage = test.loading || !test.ok;
  const request = test.responsesRequest || test.chatRequest;
  const response = test.responsesResponse || test.chatResponse;
  const summary = getApiResponsesSummary(test);

  return (
    <div className="api-test-detail-content">
      <div className="api-test-detail-head">
        <div className="api-test-detail-title-stack">
          <div className="api-test-detail-time">{timeText || t('等待预检时间')}</div>
          {lastAvailableTimeText && (
            <div className="api-test-detail-time api-test-detail-time-available">
              {lastAvailableTimeText}
            </div>
          )}
        </div>
        <span className={`api-test-panel-state ${state}`}>{getApiTestStateLabel(test, t)}</span>
      </div>

      {shouldShowMessage && (
        <div className={`api-test-message ${state}`}>
          {translateRuntimeText(test.message) || getApiTestStateLabel(test, t)}
        </div>
      )}

      {summary && <ApiResponsesSummary response={response} summary={summary} testModel={test.testModel} />}

      {(request || response) && (
        <details className="api-test-raw" open={!test.ok}>
          <summary>{t('原始请求与响应')}</summary>
          <ApiTestResponsesBlock request={request} response={response} />
        </details>
      )}
    </div>
  );
}
