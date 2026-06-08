import { useState } from 'react';
import { useApiProfilePagination } from '../hooks';
import { getErrorMessage } from '../utils/errors';
import { normalizeApiBaseUrlInput } from '../utils/appState';

export default function ApiModePage({
  activeApiProfileId,
  apiProfiles,
  onAddApiProfile,
  onDeleteApiProfile,
  onEditApiProfile,
  onOpenCodexConfigToml,
  onSwitchToApiMode,
  savingApiMode,
  switching
}) {
  const [baseUrlTests, setBaseUrlTests] = useState({});
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
  const handleTestBaseUrl = async (profile, profileId) => {
    if (baseUrlTests[profileId]?.loading) return;

    let normalizedBaseUrl = '';
    try {
      normalizedBaseUrl = normalizeApiBaseUrlInput(profile.base_url);
    } catch (err) {
      setBaseUrlTests(prev => ({
        ...prev,
        [profileId]: {
          baseUrl: profile.base_url || '',
          apiKeyPresent: Boolean(profile.api_key),
          loading: false,
          ok: false,
          message: getErrorMessage(err, 'API Base URL 格式无效')
        }
      }));
      return;
    }

    setBaseUrlTests(prev => ({
      ...prev,
      [profileId]: {
        baseUrl: normalizedBaseUrl,
        apiKeyPresent: Boolean(profile.api_key),
        loading: true,
        ok: false,
        message: '正在测试 Base URL'
      }
    }));
    try {
      const res = await window.api.testApiBaseUrl({
        baseUrl: normalizedBaseUrl,
        apiKey: profile.api_key || ''
      });
      setBaseUrlTests(prev => ({
        ...prev,
        [profileId]: {
          baseUrl: normalizedBaseUrl,
          apiKeyPresent: Boolean(profile.api_key),
          loading: false,
          ok: Boolean(res && res.ok),
          message: (res && res.message) || 'Base URL 测试完成'
        }
      }));
    } catch (err) {
      setBaseUrlTests(prev => ({
        ...prev,
        [profileId]: {
          baseUrl: normalizedBaseUrl,
          apiKeyPresent: Boolean(profile.api_key),
          loading: false,
          ok: false,
          message: getErrorMessage(err, 'Base URL 测试失败')
        }
      }));
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
                  const active = profileId === activeApiProfileId;
                  const profileName = profile.name || `API ${startIdx + index + 1}`;
                  const baseUrl = profile.base_url || '';
                  const apiKey = profile.api_key || '';
                  const rawTestForThisProfile = baseUrlTests[profileId] || null;
                  const testForThisProfile = rawTestForThisProfile
                    && rawTestForThisProfile.baseUrl === baseUrl
                    && rawTestForThisProfile.apiKeyPresent === Boolean(apiKey)
                    ? rawTestForThisProfile
                    : null;
                  const testLoading = Boolean(testForThisProfile && testForThisProfile.loading);
                  const testMessage = testForThisProfile && testForThisProfile.message ? testForThisProfile.message : '';
                  const testResultState = testForThisProfile
                    ? (testLoading ? 'loading' : (testForThisProfile.ok ? 'success' : 'error'))
                    : 'idle';
                  const testTagText = testLoading
                    ? '测试中'
                    : (testForThisProfile ? (testForThisProfile.ok ? '可用' : '不可用') : '');
                  const testButtonTitle = !baseUrl
                    ? '未配置 Base URL'
                    : (!apiKey ? '未配置 API Key' : '测试 Base URL');
                  const testDisabled = !baseUrl || !apiKey || testLoading;
                  const deleteTitle = active
                    ? '当前正在使用，不能删除'
                    : profiles.length <= 1
                      ? '至少保留一个 API'
                      : '删除配置';

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
                            <span className={`api-profile-test-tag ${testResultState}`} title={testMessage}>
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
                        </div>
                      </div>

                      <div className="account-card-footer">
                        <div className="action-btns">
                          <button
                            type="button"
                            className={`api-profile-card-test-button ${testLoading ? 'is-loading' : ''}`}
                            title={testButtonTitle}
                            aria-label="测试 Base URL"
                            aria-busy={testLoading}
                            disabled={testDisabled}
                            onClick={() => handleTestBaseUrl(profile, profileId)}
                          >
                            {testLoading && <span className="api-profile-test-spinner" aria-hidden="true" />}
                            <span>{testLoading ? '测试中' : '测试 Base URL'}</span>
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
                            disabled={active || profiles.length <= 1 || savingApiMode || switching}
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
    </div>
  );
}
