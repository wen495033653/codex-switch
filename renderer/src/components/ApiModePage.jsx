import { useEffect, useRef, useState } from 'react';
import { useApiProfilePagination } from '../hooks';
import { runningIntelligenceProgress } from '../utils/intelligenceTests';
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
import CardMoreMenu from './CardMoreMenu';
import { IntelligenceRunningTag } from './IntelligencePanel';

export default function ApiModePage({
  activeApiProfileId,
  apiModeActive,
  apiProfiles,
  apiTestResults,
  checkTab,
  intelligence,
  onAddApiProfile,
  onCheckTabChange,
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
  const {
    intelligenceRuns,
    runApiIntelligenceTests,
    selectedTestIds,
    toggleTestSelected
  } = intelligence;
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
    ? profiles.find(profile => profile.id === checkModalProfileId)
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
  const intelligenceRun = checkModalProfileId ? intelligenceRuns[checkModalProfileId] || null : null;
  const intelligenceRunning = Boolean(runningIntelligenceProgress(intelligenceRun));
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
                  const profileId = profile.id;
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
                  const intelligenceProgress = runningIntelligenceProgress(intelligenceRuns[profileId]);
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
                          {intelligenceProgress && <IntelligenceRunningTag progress={intelligenceProgress} />}
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
                              ownerType: 'api_profile',
                              ownerId: profileId,
                              ownerName: profileName,
                              ownerTypeLabel: t('API 配置')
                            })}
                          />
                        </div>
                      </div>

                      <div className="account-card-footer">
                        <div className="action-btns">
                          <button
                            type="button"
                            className={`card-text-button ${testLoading ? 'is-loading' : ''}`}
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
                            className="icon-btn"
                            title={t('编辑')}
                            aria-label={t('编辑')}
                            onClick={() => onEditApiProfile(profileId)}
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
                          <CardMoreMenu
                            items={[
                              {
                                key: 'codex',
                                label: openingThisCodexApp ? t('正在打开 Codex') : t('打开独立 Codex'),
                                disabled: !configured || openingAnyCodexApp,
                                title: configured ? '' : t('配置未完整'),
                                onSelect: () => onOpenCodexAppInstance(profileId)
                              },
                              {
                                key: 'delete',
                                label: t('删除'),
                                danger: true,
                                disabled: profiles.length <= 1 || savingApiMode || switching,
                                title: deleteTitle,
                                onSelect: () => onDeleteApiProfile(profileId)
                              }
                            ]}
                          />
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
          checkTab={checkTab}
          closeCheckModal={closeCheckModal}
          commitTestModelDraft={commitTestModelDraft}
          detailModelOptions={detailModelOptions}
          detailPlaceholderText={detailPlaceholderText}
          detailProfile={detailProfile}
          detailProfileName={detailProfileName}
          detailTest={detailTest}
          effectiveDetailModel={effectiveDetailModel}
          handleTestBaseUrl={handleTestBaseUrl}
          intelligenceRun={intelligenceRun}
          intelligenceRunning={intelligenceRunning}
          modelDropdownOpen={modelDropdownOpen}
          modelsDetailAvailable={modelsDetailAvailable}
          modelsDetailLabel={modelsDetailLabel}
          modelsDetailsOpen={modelsDetailsOpen}
          modelsStatus={modelsStatus}
          runIntelligenceTests={runApiIntelligenceTests}
          selectedTestIds={selectedTestIds}
          selectTestModelDraft={selectTestModelDraft}
          setCheckTab={onCheckTabChange}
          setModelDropdownOpen={setModelDropdownOpen}
          setModelsDetailsOpen={setModelsDetailsOpen}
          toggleTestSelected={toggleTestSelected}
          visibleDetailTest={visibleDetailTest}
        />
      )}
    </div>
  );
}
