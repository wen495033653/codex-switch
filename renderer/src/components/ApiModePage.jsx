import { useEffect, useRef, useState } from 'react';
import { useApiProfilePagination } from '../hooks';
import { getCodexAppInstanceKey } from '../utils/codexAppInstances';
import {
  DEFAULT_API_TEST_MODEL,
  getApiTestSignature,
  isFreshApiTest,
  normalizeApiTestModelInput,
  normalizeApiTestResults,
  runApiProfilePrecheck,
  withInFlightApiTests,
} from '../utils/apiPrecheck';
import Pagination from './Pagination';
import UsageStatsSummary from './UsageStatsSummary';
import { useI18n } from '../i18n';
import {
  getApiCheckTimeText,
  getApiLastAvailableTimeText,
  getApiTestModelOptions,
  getApiModelsStatus,
  getApiTestState,
  getApiTestStateLabel,
  getApiModelsDetailLabel,
} from '../utils/apiTestView';
import ApiCheckModal from './api/ApiCheckModal';

export default function ApiModePage({
  activeApiProfileId,
  apiModeActive,
  apiProfiles,
  apiTestResults,
  onAddApiProfile,
  onDeleteApiProfile,
  onEditApiProfile,
  onOpenUsageStatsDetail,
  onOpenCodexAppInstance,
  openingCodexAppTarget,
  runningCodexAppInstances,
  onSaveApiTestResults,
  onSwitchToApiMode,
  savingApiMode,
  switching,
  usageStatsByApiProfile
}) {
  const { language, t, translateRuntimeText } = useI18n();
  const [baseUrlTests, setBaseUrlTests] = useState(() => normalizeApiTestResults(apiTestResults));
  const baseUrlTestsRef = useRef(normalizeApiTestResults(apiTestResults));
  const inFlightProfileIdsRef = useRef(new Set());
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
  const modelsStatus = getApiModelsStatus(detailTest, t);
  const modelsDetailAvailable = Boolean(detailTest && detailTest.modelsResponse);
  const modelsDetailLabel = getApiModelsDetailLabel(detailTest, t);
  const detailTestMatchesModel = Boolean(detailTest && detailTest.testModel === effectiveDetailModel);
  const visibleDetailTest = detailTestMatchesModel ? detailTest : null;
  const detailPlaceholderText = detailTest && !detailTestMatchesModel
    ? t('更改测试模型后需要重新预检')
    : t('准备预检');

  useEffect(() => {
    const nextResults = withInFlightApiTests(
      normalizeApiTestResults(apiTestResults),
      baseUrlTestsRef.current,
      inFlightProfileIdsRef.current
    );
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
      onSaveApiTestResults(normalizeApiTestResults(nextResults));
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
    const inFlightProfileIds = inFlightProfileIdsRef.current;
    if (inFlightProfileIds.has(profileId)) return;
    inFlightProfileIds.add(profileId);

    try {
      const testModel = normalizeApiTestModelInput(rawTestModel);
      const result = await runApiProfilePrecheck({
        profile,
        profileName,
        model: testModel,
        previousTest: baseUrlTestsRef.current[profileId],
        onUpdate: test => setApiTestForProfile(profileId, test),
        testApiBaseUrl: payload => window.api.testApiBaseUrl(payload)
      });
      setApiTestForProfile(profileId, result, true);
    } finally {
      inFlightProfileIds.delete(profileId);
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
              className="btn btn-primary api-profile-add-button"
              onClick={onAddApiProfile}
              disabled={savingApiMode || switching}
            >
              <span className="btn-leading-icon" aria-hidden="true">+</span>
              <span>{t('新增 API')}</span>
            </button>
            </div>
            <div className="list-panel api-profile-panel">
              <div className="account-grid api-profile-grid" ref={apiProfileGridRef} role="list" aria-label={t('API 配置列表')}>
                {currentItems.map((profile, index) => {
                  const profileId = profile.id || `api-${startIdx + index}`;
                  const configured = Boolean(profile.name && profile.base_url && profile.api_key);
                  const codexAppTargetKey = `api:${profileId}`;
                  const codexAppInstanceKey = getCodexAppInstanceKey('api', profileId);
                  const openingThisCodexApp = openingCodexAppTarget === codexAppTargetKey;
                  const openingAnyCodexApp = Boolean(openingCodexAppTarget);
                  const codexAppInstanceRunning = Boolean(
                    codexAppInstanceKey && runningCodexAppInstances && runningCodexAppInstances[codexAppInstanceKey]
                  );
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
                  const testTagText = testForThisProfile ? getApiTestStateLabel(testForThisProfile, t) : '';
                  const testTimeText = getApiCheckTimeText(testForThisProfile, t, language);
                  const lastAvailableTimeText = getApiLastAvailableTimeText(testForThisProfile, t, language);
                  const hasFreshTest = isFreshApiTest(testForThisProfile);
                  const testButtonTitle = !baseUrl
                    ? t('未配置 Base URL')
                    : (!apiKey
                        ? t('未配置 API Key')
                        : (hasFreshTest
                            ? t('1 小时内已预检，点击查看详情')
                            : t('使用 {model} 预检 API', { model: normalizedTestModel })));
                  const testDisabled = !baseUrl || !apiKey || testLoading;
                  const testActionText = testLoading ? t('预检中') : t('预检');
                  const deleteTitle = profiles.length <= 1
                    ? t('至少保留一个 API')
                    : t('删除配置');
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
                          {active && <span className="current-badge">{t('当前')}</span>}
                          {codexAppInstanceRunning && (
                            <span className="codex-app-running-badge" title={t('独立 Codex 正在运行')}>
                              {t('窗口运行中')}
                            </span>
                          )}
                          {testForThisProfile && (
                            <span className={`api-profile-test-tag ${testResultState}`} title={lastAvailableTimeText || testTimeText || translateRuntimeText(testForThisProfile.message) || testTagText}>
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
                              title={baseUrl || t('未配置 Base URL')}
                            >
                              {baseUrl || t('未配置 Base URL')}
                            </span>
                          </div>
                          <UsageStatsSummary
                            stats={usageStats}
                            onOpenDetails={() => onOpenUsageStatsDetail?.({
                              ownerName: profileName,
                              ownerTypeLabel: t('API 配置'),
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
                            aria-label={t('预检 API')}
                            aria-busy={testLoading}
                            disabled={testDisabled}
                            onClick={() => openCheckModalAndRun(profile, profileId, profileName, normalizedTestModel)}
                          >
                            {testLoading && <span className="api-profile-test-spinner" aria-hidden="true" />}
                            <span>{testActionText}</span>
                          </button>
                          <button
                            type="button"
                            className={`icon-btn ${codexAppInstanceRunning ? 'codex-app-instance-running' : ''}`}
                            title={configured ? (openingThisCodexApp ? t('正在打开 Codex') : (codexAppInstanceRunning ? t('打开独立 Codex 窗口') : t('用此 API 打开独立 Codex'))) : t('配置未完整')}
                            aria-label={openingThisCodexApp ? t('正在打开 Codex') : (codexAppInstanceRunning ? t('打开独立 Codex 窗口') : t('用此 API 打开独立 Codex'))}
                            disabled={!configured || openingAnyCodexApp}
                            onClick={() => onOpenCodexAppInstance(profileId)}
                          >
                            <svg className={openingThisCodexApp ? 'icon-spin' : ''} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7.5h8.5A2.5 2.5 0 0 1 19 10v6.5A2.5 2.5 0 0 1 16.5 19H8a2.5 2.5 0 0 1-2.5-2.5V10A2.5 2.5 0 0 1 8 7.5Z" />
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5.5 15.5H5A2.5 2.5 0 0 1 2.5 13V6.5A2.5 2.5 0 0 1 5 4h8.5A2.5 2.5 0 0 1 16 6.5V7" />
                            </svg>
                          </button>
                          <button
                            type="button"
                            className="icon-btn"
                            title={t('编辑此配置')}
                            aria-label={t('编辑此配置')}
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
                              title={configured ? t('切换到此 API') : t('配置未完整')}
                              aria-label={configured ? t('切换到此 API') : t('配置未完整')}
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
                            aria-label={deleteTitle}
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
                  <div className="empty-state empty-state-card">{t('暂无 API 配置')}</div>
                )}
              </div>

              <Pagination
                hideSinglePage
                onPageChange={setPage}
                page={page}
                pageSize={pageSize}
                startIdx={startIdx}
                total={total}
                totalPages={totalPages}
              />
            </div>
          </div>
        </div>
      </div>

      {checkModalProfileId && (
        <ApiCheckModal
          checkModalProfileId={checkModalProfileId}
          closeCheckModal={closeCheckModal}
          commitTestModelDraft={commitTestModelDraft}
          detailModelOptions={detailModelOptions}
          detailPlaceholderText={detailPlaceholderText}
          detailProfile={detailProfile}
          detailProfileName={detailProfileName}
          detailTest={detailTest}
          effectiveDetailModel={effectiveDetailModel}
          handleTestBaseUrl={handleTestBaseUrl}
          modelDropdownOpen={modelDropdownOpen}
          modelsDetailAvailable={modelsDetailAvailable}
          modelsDetailLabel={modelsDetailLabel}
          modelsDetailsOpen={modelsDetailsOpen}
          modelsStatus={modelsStatus}
          selectTestModelDraft={selectTestModelDraft}
          setModelDropdownOpen={setModelDropdownOpen}
          setModelsDetailsOpen={setModelsDetailsOpen}
          visibleDetailTest={visibleDetailTest}
        />
      )}
    </div>
  );
}
