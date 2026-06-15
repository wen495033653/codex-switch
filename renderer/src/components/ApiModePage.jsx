import { useEffect, useRef, useState } from 'react';
import { useApiProfilePagination } from '../hooks';
import { getErrorMessage } from '../utils/errors';
import { normalizeApiBaseUrlInput } from '../utils/appState';
import Modal from './Modal';
import UsageStatsSummary from './UsageStatsSummary';

const DEFAULT_API_TEST_MODEL = 'gpt-5.5';
const API_TEST_CACHE_TTL_MS = 60 * 60 * 1000;

function normalizeApiTestModelInput(value) {
  const trimmed = String(value || '').trim();
  return trimmed || DEFAULT_API_TEST_MODEL;
}

function getApiKeyFingerprint(value) {
  const text = String(value || '');
  let hash = 2166136261;
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return `${text.length}:${(hash >>> 0).toString(16)}`;
}

function getApiTestSignature(baseUrl, apiKey, model) {
  return [
    String(baseUrl || ''),
    getApiKeyFingerprint(apiKey),
    normalizeApiTestModelInput(model)
  ].join('\n');
}

function normalizeApiTestResults(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter(([, item]) => item && typeof item === 'object' && !Array.isArray(item))
  );
}

function isFreshApiTest(test, now = Date.now()) {
  return Boolean(
    test
    && !test.loading
    && Number.isFinite(test.checkedAt)
    && now - test.checkedAt < API_TEST_CACHE_TTL_MS
  );
}

function formatApiCheckTime(value) {
  if (!Number.isFinite(value)) return '';
  try {
    return new Intl.DateTimeFormat('zh-CN', {
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hour12: false
    }).format(new Date(value));
  } catch {
    return '';
  }
}

function getApiCheckTimeText(test) {
  if (!test) return '';
  const timestamp = Number.isFinite(test.checkedAt) ? test.checkedAt : test.startedAt;
  const formatted = formatApiCheckTime(timestamp);
  if (!formatted) return '';
  return `${test.loading ? '开始时间' : '检查时间'} ${formatted}`;
}

function getApiLastAvailableAt(test) {
  if (!test) return null;
  if (Number.isFinite(test.lastAvailableAt)) return test.lastAvailableAt;
  if (test.ok && Number.isFinite(test.checkedAt)) return test.checkedAt;
  return null;
}

function getApiLastAvailableTimeText(test) {
  const timestamp = getApiLastAvailableAt(test);
  const formatted = formatApiCheckTime(timestamp);
  return formatted ? `上次可用 ${formatted}` : '';
}

function getApiTestModelOptions(test, fallbackModel) {
  const models = Array.isArray(test && test.modelIds)
    ? test.modelIds.map(item => String(item || '').trim()).filter(Boolean)
    : [];
  const uniqueModels = [...new Set(models)];
  const fallback = normalizeApiTestModelInput(fallbackModel);
  if (uniqueModels.length > 0) {
    return uniqueModels.includes(fallback) ? uniqueModels : [fallback, ...uniqueModels];
  }
  return [fallback];
}

function getApiModelsStatus(test) {
  if (!test) {
    return { state: 'idle', text: '等待 /models' };
  }
  if (test.loading) {
    return { state: 'loading', text: '正在获取 /models' };
  }
  const response = test.modelsResponse || null;
  if (response && response.error) {
    return { state: 'error', text: '/models 请求失败' };
  }
  const status = response && Number.isFinite(response.status) ? response.status : null;
  if (status && (status < 200 || status >= 300)) {
    return { state: 'error', text: `/models HTTP ${status}` };
  }
  const models = Array.isArray(test.modelIds)
    ? test.modelIds.map(item => String(item || '').trim()).filter(Boolean)
    : [];
  if (models.length > 0) {
    if (!models.includes(DEFAULT_API_TEST_MODEL) && test.testModel === DEFAULT_API_TEST_MODEL) {
      return { state: 'warning', text: `/models：${models.length} 个模型，默认模型不在列表` };
    }
    return { state: 'success', text: `/models：${models.length} 个模型` };
  }
  if (response) {
    return { state: 'warning', text: '/models 未返回可选模型' };
  }
  return { state: 'idle', text: '等待 /models' };
}

function getApiTestState(test) {
  if (!test) return 'idle';
  if (test.loading) return 'loading';
  return test.ok ? 'success' : 'error';
}

function getApiTestStateLabel(test) {
  const state = getApiTestState(test);
  if (state === 'loading') return '检查中';
  if (state === 'success') return '可用';
  if (state === 'error') return '不可用';
  return '未检查';
}

function prettyApiTestBody(body) {
  if (typeof body !== 'string' || !body) return '';
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}

function formatApiTestJson(value) {
  if (value === null || value === undefined) return '';
  return JSON.stringify(value, null, 2);
}

function getApiTestResponseBody(response) {
  if (!response) return '';
  if (typeof response.body === 'string' && response.body) return prettyApiTestBody(response.body);
  if (response.json !== null && response.json !== undefined) return formatApiTestJson(response.json);
  if (response.error) return String(response.error);
  return '空响应';
}

function getApiTestResponseStatusLabel(response) {
  if (!response) return '未请求';
  const status = Number.isFinite(response.status) ? response.status : null;
  const statusText = response.statusText ? ` ${response.statusText}` : '';
  return status ? `HTTP ${status}${statusText}` : (response.error ? '请求失败' : '无状态');
}

function getApiModelsDetailLabel(test) {
  const response = test && test.modelsResponse ? test.modelsResponse : null;
  if (!response) return '/models';
  const status = Number.isFinite(response.status) ? response.status : null;
  if (status) return `/models ${status}`;
  if (response.error) return '/models 失败';
  return '/models 详情';
}

function ApiTestResponseBlock({ response, title }) {
  if (!response) return null;
  const body = getApiTestResponseBody(response);

  return (
    <section className="api-test-response-block">
      <div className="api-test-response-head">
        <div className="api-test-response-title">{title}</div>
        <div className="api-test-response-status">{getApiTestResponseStatusLabel(response)}</div>
      </div>
      <div className="api-test-response-endpoint" title={response.endpoint || ''}>
        {response.endpoint || '未返回 endpoint'}
      </div>
      <pre className="api-test-response-body">{body}</pre>
    </section>
  );
}

function ApiTestChatBlock({ request, response }) {
  if (!request && !response) return null;
  const endpoint = (response && response.endpoint) || (request && request.endpoint) || '';
  const requestBody = request ? formatApiTestJson(request.body || {}) : '未发送请求';
  const responseBody = response ? getApiTestResponseBody(response) : '未返回响应';

  return (
    <section className="api-test-response-block api-test-chat-block">
      <div className="api-test-response-head">
        <div className="api-test-response-title">Chat 调用</div>
        <div className="api-test-response-status">{getApiTestResponseStatusLabel(response)}</div>
      </div>
      <div className="api-test-response-endpoint" title={endpoint}>
        {endpoint || '未返回 endpoint'}
      </div>
      <div className="api-test-chat-grid">
        <div className="api-test-chat-pane">
          <div className="api-test-chat-pane-title">请求</div>
          <pre className="api-test-response-body">{requestBody}</pre>
        </div>
        <div className="api-test-chat-pane">
          <div className="api-test-chat-pane-title">返回</div>
          <pre className="api-test-response-body">{responseBody}</pre>
        </div>
      </div>
    </section>
  );
}

function ApiTestDetailContent({ test }) {
  const state = getApiTestState(test);
  const timeText = getApiCheckTimeText(test);
  const lastAvailableTimeText = getApiLastAvailableTimeText(test);
  const shouldShowMessage = test.loading || !test.ok;

  return (
    <div className="api-test-detail-content">
      <div className="api-test-detail-head">
        <div className="api-test-detail-title-stack">
          <div className="api-test-detail-time">{timeText || '等待检查时间'}</div>
          {lastAvailableTimeText && (
            <div className="api-test-detail-time api-test-detail-time-available">
              {lastAvailableTimeText}
            </div>
          )}
        </div>
        <span className={`api-test-panel-state ${state}`}>{getApiTestStateLabel(test)}</span>
      </div>

      {shouldShowMessage && (
        <div className={`api-test-message ${state}`}>
          {test.message || getApiTestStateLabel(test)}
        </div>
      )}

      <ApiTestChatBlock request={test.chatRequest} response={test.chatResponse} />
    </div>
  );
}

export default function ApiModePage({
  activeApiProfileId,
  apiModeActive,
  apiProfiles,
  apiTestResults,
  onAddApiProfile,
  onDeleteApiProfile,
  onEditApiProfile,
  onOpenCodexConfigToml,
  onOpenUsageStatsDetail,
  onSaveApiTestResults,
  onSwitchToApiMode,
  savingApiMode,
  switching,
  usageStatsByApiProfile
}) {
  const [baseUrlTests, setBaseUrlTests] = useState(() => normalizeApiTestResults(apiTestResults));
  const baseUrlTestsRef = useRef(normalizeApiTestResults(apiTestResults));
  const [testModelDrafts, setTestModelDrafts] = useState({});
  const [checkModalProfileId, setCheckModalProfileId] = useState(null);
  const [modelDropdownOpen, setModelDropdownOpen] = useState(false);
  const [modelsDetailsOpen, setModelsDetailsOpen] = useState(false);
  const profiles = Array.isArray(apiProfiles) && apiProfiles.length > 0
    ? apiProfiles
    : [];
  const {
    apiProfileGridRef,
    currentItems,
    page,
    pageSize,
    setPage,
    startIdx,
    total,
    totalPages
  } = useApiProfilePagination({
    activeId: activeApiProfileId,
    profiles
  });
  const detailTest = checkModalProfileId ? baseUrlTests[checkModalProfileId] || null : null;
  const detailProfile = checkModalProfileId
    ? profiles.find((profile, index) => (profile.id || `api-${index}`) === checkModalProfileId)
    : null;
  const detailProfileName = detailProfile ? detailProfile.name || checkModalProfileId : '';
  const detailModelValue = checkModalProfileId
    ? testModelDrafts[checkModalProfileId] ?? (detailTest && detailTest.testModel) ?? DEFAULT_API_TEST_MODEL
    : DEFAULT_API_TEST_MODEL;
  const detailModelOptions = getApiTestModelOptions(detailTest, detailModelValue);
  const normalizedDetailModel = normalizeApiTestModelInput(detailModelValue);
  const effectiveDetailModel = detailModelOptions.includes(normalizedDetailModel)
    ? normalizedDetailModel
    : detailModelOptions[0];
  const modelsStatus = getApiModelsStatus(detailTest);
  const modelsDetailAvailable = Boolean(detailTest && detailTest.modelsResponse);
  const modelsDetailLabel = getApiModelsDetailLabel(detailTest);
  const detailTestMatchesModel = Boolean(detailTest && detailTest.testModel === effectiveDetailModel);
  const visibleDetailTest = detailTestMatchesModel ? detailTest : null;
  const detailPlaceholderText = detailTest && !detailTestMatchesModel
    ? '更改测试模型后需要重新检查'
    : '准备检查';

  useEffect(() => {
    const nextResults = normalizeApiTestResults(apiTestResults);
    baseUrlTestsRef.current = nextResults;
    setBaseUrlTests(nextResults);
  }, [apiTestResults]);

  const setApiTestForProfile = (profileId, test, shouldPersist = false) => {
    const nextResults = {
      ...baseUrlTestsRef.current,
      [profileId]: test
    };
    baseUrlTestsRef.current = nextResults;
    setBaseUrlTests(nextResults);
    if (shouldPersist && typeof onSaveApiTestResults === 'function') {
      onSaveApiTestResults(nextResults);
    }
  };

  const updateTestModelDraft = (profileId, value) => {
    setTestModelDrafts(prev => ({
      ...prev,
      [profileId]: value
    }));
  };

  const selectTestModelDraft = (profileId, value) => {
    setTestModelDrafts(prev => ({
      ...prev,
      [profileId]: normalizeApiTestModelInput(value)
    }));
  };

  const commitTestModelDraft = (profileId) => {
    setTestModelDrafts(prev => ({
      ...prev,
      [profileId]: normalizeApiTestModelInput(prev[profileId])
    }));
  };

  const closeCheckModal = () => {
    setCheckModalProfileId(null);
    setModelDropdownOpen(false);
    setModelsDetailsOpen(false);
  };

  const openCheckModalAndRun = (profile, profileId, profileName, rawTestModel) => {
    const testModel = normalizeApiTestModelInput(rawTestModel);
    const signature = getApiTestSignature(profile.base_url || '', profile.api_key || '', testModel);
    const existingTest = baseUrlTestsRef.current[profileId] || null;
    setCheckModalProfileId(profileId);
    setModelDropdownOpen(false);
    setModelsDetailsOpen(false);
    updateTestModelDraft(profileId, testModel);
    if (existingTest && existingTest.signature === signature && isFreshApiTest(existingTest)) {
      return;
    }
    handleTestBaseUrl(profile, profileId, profileName, testModel);
  };

  const handleTestBaseUrl = async (profile, profileId, profileName, rawTestModel) => {
    if (baseUrlTestsRef.current[profileId]?.loading) return;

    const sourceBaseUrl = profile.base_url || '';
    const testModel = normalizeApiTestModelInput(rawTestModel);
    const signature = getApiTestSignature(sourceBaseUrl, profile.api_key || '', testModel);
    const startedAt = Date.now();
    const previousLastAvailableAt = getApiLastAvailableAt(baseUrlTestsRef.current[profileId]);
    let normalizedBaseUrl = '';
    try {
      normalizedBaseUrl = normalizeApiBaseUrlInput(sourceBaseUrl);
    } catch (err) {
      const checkedAt = Date.now();
      setApiTestForProfile(profileId, {
        signature,
        sourceBaseUrl,
        baseUrl: sourceBaseUrl,
        apiKeyPresent: Boolean(profile.api_key),
        testModel,
        profileName,
        stage: 'input',
        startedAt,
        checkedAt,
        lastAvailableAt: previousLastAvailableAt,
        loading: false,
        ok: false,
        message: getErrorMessage(err, 'API Base URL 格式无效')
      }, true);
      return;
    }

    setApiTestForProfile(profileId, {
      signature,
      sourceBaseUrl,
      baseUrl: normalizedBaseUrl,
      apiKeyPresent: Boolean(profile.api_key),
      testModel,
      profileName,
      stage: 'models',
      startedAt,
      lastAvailableAt: previousLastAvailableAt,
      loading: true,
      ok: false,
      message: '正在检查 API'
    });
    try {
      const res = await window.api.testApiBaseUrl({
        baseUrl: normalizedBaseUrl,
        apiKey: profile.api_key || '',
        model: testModel
      });
      const result = res && typeof res === 'object' ? res : {};
      const checkedAt = Date.now();
      setApiTestForProfile(profileId, {
        ...result,
        signature,
        sourceBaseUrl,
        baseUrl: normalizedBaseUrl,
        apiKeyPresent: Boolean(profile.api_key),
        testModel: result.testModel || testModel,
        profileName,
        startedAt,
        checkedAt,
        lastAvailableAt: result.ok ? checkedAt : previousLastAvailableAt,
        loading: false,
        ok: Boolean(result.ok),
        message: result.message || 'API 检查完成'
      }, true);
    } catch (err) {
      const checkedAt = Date.now();
      setApiTestForProfile(profileId, {
        signature,
        sourceBaseUrl,
        baseUrl: normalizedBaseUrl,
        apiKeyPresent: Boolean(profile.api_key),
        testModel,
        profileName,
        stage: 'chat',
        startedAt,
        checkedAt,
        lastAvailableAt: previousLastAvailableAt,
        loading: false,
        ok: false,
        message: getErrorMessage(err, 'API 检查失败')
      }, true);
    }
  };

  return (
    <div className="api-mode-page">
      <div className="api-console-grid">
        <div className="api-config-stack">
          <div className="api-config-cluster">
            <div className="api-page-actions">
              <button
                type="button"
                className="btn btn-secondary api-config-open-button"
                onClick={onOpenCodexConfigToml}
              >
                打开 config.toml
              </button>
              <button
                type="button"
                className="btn btn-primary api-profile-add-button"
                onClick={onAddApiProfile}
                disabled={savingApiMode || switching}
              >
                + 新增 API
              </button>
            </div>

            <div className="list-panel api-profile-panel">
              <div className="account-grid api-profile-grid" ref={apiProfileGridRef} role="list" aria-label="API 配置列表">
                {currentItems.map((profile, index) => {
                  const profileId = profile.id || `api-${startIdx + index}`;
                  const configured = Boolean(profile.name && profile.base_url && profile.api_key);
                  const active = apiModeActive && profileId === activeApiProfileId;
                  const profileName = profile.name || `API ${startIdx + index + 1}`;
                  const baseUrl = profile.base_url || '';
                  const apiKey = profile.api_key || '';
                  const rawTestForThisProfile = baseUrlTests[profileId] || null;
                  const normalizedTestModel = normalizeApiTestModelInput(
                    testModelDrafts[profileId] ?? (rawTestForThisProfile && rawTestForThisProfile.testModel)
                  );
                  const testSignature = getApiTestSignature(baseUrl, apiKey, normalizedTestModel);
                  const testForThisProfile = rawTestForThisProfile
                    && rawTestForThisProfile.signature === testSignature
                    ? rawTestForThisProfile
                    : null;
                  const testLoading = Boolean(testForThisProfile && testForThisProfile.loading);
                  const testResultState = testForThisProfile ? getApiTestState(testForThisProfile) : 'idle';
                  const testTagText = testForThisProfile ? getApiTestStateLabel(testForThisProfile) : '';
                  const testTimeText = getApiCheckTimeText(testForThisProfile);
                  const lastAvailableTimeText = getApiLastAvailableTimeText(testForThisProfile);
                  const hasFreshTest = isFreshApiTest(testForThisProfile);
                  const testButtonTitle = !baseUrl
                    ? '未配置 Base URL'
                    : (!apiKey
                        ? '未配置 API Key'
                        : (hasFreshTest
                            ? '1 小时内已检查，点击查看详情'
                            : `使用 ${normalizedTestModel} 检查 API`));
                  const testDisabled = !baseUrl || !apiKey || testLoading;
                  const testActionText = testLoading ? '检查中' : '检查';
                  const deleteTitle = profiles.length <= 1
                    ? '至少保留一个 API'
                    : '删除配置';
                  const usageStats = usageStatsByApiProfile
                    ? usageStatsByApiProfile[profileId]
                    : null;

                  return (
                    <div
                      key={profileId}
                      className={`account-card api-profile-card ${active ? 'active' : ''}`}
                      role="listitem"
                    >
                      <div className="account-card-head">
                        <div className="account-card-name-row">
                          <div className="account-card-name" title={profileName}>{profileName}</div>
                          {active && <span className="current-badge">当前</span>}
                          {testForThisProfile && (
                            <span className={`api-profile-test-tag ${testResultState}`} title={lastAvailableTimeText || testTimeText || testForThisProfile.message || testTagText}>
                              {testLoading && <span className="api-profile-test-spinner" aria-hidden="true" />}
                              <span>{testTagText}</span>
                            </span>
                          )}
                        </div>
                      </div>

                      <div className="account-card-body">
                        <div className="api-profile-card-info">
                          <div className="api-profile-card-info-row">
                            <span className="api-profile-card-label">Base URL</span>
                            <span
                              className={`api-profile-card-value ${baseUrl ? '' : 'muted'}`}
                              title={baseUrl || '未配置 Base URL'}
                            >
                              {baseUrl || '未配置 Base URL'}
                            </span>
                          </div>
                          <UsageStatsSummary
                            stats={usageStats}
                            onOpenDetails={() => onOpenUsageStatsDetail?.({
                              ownerName: profileName,
                              ownerTypeLabel: 'API 配置',
                              stats: usageStats
                            })}
                          />
                        </div>
                      </div>

                      <div className="account-card-footer">
                        <div className="action-btns">
                          <button
                            type="button"
                            className={`api-profile-card-test-button ${testLoading ? 'is-loading' : ''}`}
                            title={testButtonTitle}
                            aria-label="检查 API"
                            aria-busy={testLoading}
                            disabled={testDisabled}
                            onClick={() => openCheckModalAndRun(profile, profileId, profileName, normalizedTestModel)}
                          >
                            {testLoading && <span className="api-profile-test-spinner" aria-hidden="true" />}
                            <span>{testActionText}</span>
                          </button>
                          <button
                            type="button"
                            className="icon-btn"
                            title="编辑此配置"
                            onClick={event => {
                              event.stopPropagation();
                              onEditApiProfile(profileId);
                            }}
                          >
                            <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="m16.862 4.487 1.688-1.688a1.875 1.875 0 1 1 2.652 2.652L10.582 16.07a4.5 4.5 0 0 1-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 0 1 1.13-1.897l8.932-8.931Z" />
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19.5 7.125 16.875 4.5M18 14v4.75A2.25 2.25 0 0 1 15.75 21h-10.5A2.25 2.25 0 0 1 3 18.75V8.25A2.25 2.25 0 0 1 5.25 6H10" />
                            </svg>
                          </button>
                          {!active && (
                            <button
                              type="button"
                              className="icon-btn"
                              title={configured ? '切换到此 API' : '配置未完整'}
                              disabled={!configured || savingApiMode || switching}
                              onClick={() => onSwitchToApiMode(profileId)}
                            >
                              ⚡
                            </button>
                          )}
                          <button
                            type="button"
                            className="icon-btn danger"
                            title={deleteTitle}
                            disabled={profiles.length <= 1 || savingApiMode || switching}
                            onClick={() => onDeleteApiProfile(profileId)}
                          >
                            <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 7h12m-9 0V5.75A1.75 1.75 0 0 1 10.75 4h2.5A1.75 1.75 0 0 1 15 5.75V7m-7.75 0 .75 12.25A1.75 1.75 0 0 0 9.75 21h4.5A1.75 1.75 0 0 0 16 19.25L16.75 7M10 11v6m4-6v6" />
                            </svg>
                          </button>
                        </div>
                      </div>
                    </div>
                  );
                })}

                {currentItems.length === 0 && (
                  <div className="empty-state empty-state-card">暂无 API 配置</div>
                )}
              </div>

              <div className="panel-footer">
                <div className="footer-info">
                  显示第 {total === 0 ? 0 : startIdx + 1} 到 {Math.min(startIdx + pageSize, total)} 条，共 {total} 条
                </div>
                {totalPages > 1 && (
                  <div className="pagination">
                    <button className="page-btn" disabled={page === 1} onClick={() => setPage(Math.max(1, page - 1))}>
                      &lt;
                    </button>
                    {Array.from({ length: totalPages }, (_, i) => i + 1).map(item => (
                      <button key={item} className={`page-btn ${page === item ? 'active' : ''}`} onClick={() => setPage(item)}>
                        {item}
                      </button>
                    ))}
                    <button className="page-btn" disabled={page === totalPages} onClick={() => setPage(Math.min(totalPages, page + 1))}>
                      &gt;
                    </button>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      </div>

      {checkModalProfileId && (
        <Modal
          title={detailProfileName || checkModalProfileId || 'API 检查'}
          width="760px"
          onClose={closeCheckModal}
        >
          <div className="api-check-controls">
            <div className="api-check-model-field">
              <span>测试模型</span>
              <div
                className="api-check-model-picker"
                onBlur={event => {
                  if (!event.currentTarget.contains(event.relatedTarget)) {
                    setModelDropdownOpen(false);
                    commitTestModelDraft(checkModalProfileId);
                  }
                }}
              >
                <button
                  type="button"
                  className="api-check-model-trigger"
                  disabled={Boolean(detailTest && detailTest.loading)}
                  onClick={() => setModelDropdownOpen(open => !open)}
                  aria-haspopup="listbox"
                  aria-expanded={modelDropdownOpen}
                >
                  <span title={effectiveDetailModel}>{effectiveDetailModel}</span>
                  <svg aria-hidden="true" viewBox="0 0 20 20" fill="currentColor">
                    <path fillRule="evenodd" d="M5.23 7.21a.75.75 0 0 1 1.06.02L10 11.168l3.71-3.938a.75.75 0 1 1 1.08 1.04l-4.25 4.5a.75.75 0 0 1-1.08 0l-4.25-4.5a.75.75 0 0 1 .02-1.06Z" clipRule="evenodd" />
                  </svg>
                </button>
                {modelDropdownOpen && !(detailTest && detailTest.loading) && (
                  <div className="api-check-model-menu" role="listbox">
                    {detailModelOptions.map(model => {
                      const selected = model === effectiveDetailModel;
                      return (
                        <button
                          key={model}
                          type="button"
                          className={`api-check-model-option ${selected ? 'selected' : ''}`}
                          role="option"
                          aria-selected={selected}
                          onClick={() => {
                            selectTestModelDraft(checkModalProfileId, model);
                            setModelDropdownOpen(false);
                          }}
                        >
                          {model}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
              <div className="api-check-model-meta">
                <div className={`api-check-model-status ${modelsStatus.state}`}>
                  {modelsStatus.text}
                </div>
                {modelsDetailAvailable && (
                  <button
                    type="button"
                    className={`api-check-model-tag ${modelsStatus.state} ${modelsDetailsOpen ? 'active' : ''}`}
                    onClick={() => setModelsDetailsOpen(open => !open)}
                    aria-expanded={modelsDetailsOpen}
                  >
                    {modelsDetailLabel}
                  </button>
                )}
              </div>
            </div>
          </div>

          {modelsDetailsOpen && modelsDetailAvailable && (
            <div className="api-check-model-detail">
              <ApiTestResponseBlock title="/models 详情" response={detailTest.modelsResponse} />
            </div>
          )}

          {visibleDetailTest ? (
            <ApiTestDetailContent test={visibleDetailTest} />
          ) : (
            <div className="api-check-placeholder">{detailPlaceholderText}</div>
          )}

          <div className="api-test-detail-actions">
            <button
              type="button"
              className="btn btn-primary api-test-detail-retest-button"
              disabled={!detailProfile || Boolean(detailTest && detailTest.loading)}
              onClick={() => handleTestBaseUrl(
                detailProfile,
                checkModalProfileId,
                detailProfile.name || (detailTest && detailTest.profileName) || checkModalProfileId,
                effectiveDetailModel
              )}
            >
              重新检查
            </button>
            <button
              type="button"
              className="btn btn-secondary api-test-detail-close-button"
              onClick={closeCheckModal}
            >
              关闭
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}
