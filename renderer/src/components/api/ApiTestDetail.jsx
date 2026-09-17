import { useI18n } from '../../i18n';
import { getApiCheckTimeText, getApiLastAvailableTimeText, getApiTestState, getApiTestStateLabel, formatApiTestJson, getApiTestResponseBody, getApiTestResponseStatusLabel } from '../../utils/apiTestView';

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

export default function ApiTestDetailContent({ test }) {
  const { language, t, translateRuntimeText } = useI18n();
  const state = getApiTestState(test);
  const timeText = getApiCheckTimeText(test, t, language);
  const lastAvailableTimeText = getApiLastAvailableTimeText(test, t, language);
  const shouldShowMessage = test.loading || !test.ok;

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

      <ApiTestResponsesBlock
        request={test.responsesRequest || test.chatRequest}
        response={test.responsesResponse || test.chatResponse}
      />
    </div>
  );
}
