import { useEffect, useState } from 'react';
import { useApiProfilePagination } from '../hooks';
import { API_PROMO_CONFIG_URL } from '../utils/appState';

export default function ApiModePage({
  activeApiProfileId,
  apiProfiles,
  apiPromoBarOpen,
  onAddApiProfile,
  onConfigureGptPoolApi,
  onDeleteApiProfile,
  onEditApiProfile,
  onOpenCodexConfigToml,
  onOpenGptPool,
  onSetApiPromoBarOpen,
  onSwitchToApiMode,
  savingApiMode,
  switching
}) {
  const [apiPromoEnabled, setApiPromoEnabled] = useState(true);
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

  useEffect(() => {
    let cancelled = false;

    const loadApiPromoConfig = async () => {
      try {
        const response = await fetch(`${API_PROMO_CONFIG_URL}?t=${Date.now()}`, {
          cache: 'no-store'
        });
        if (!response.ok) return;

        const config = await response.json();
        if (!cancelled && config && config.apiPromo && config.apiPromo.enabled === false) {
          setApiPromoEnabled(false);
        }
      } catch (_err) {
        // Keep the bundled promo unless the remote config explicitly disables it.
      }
    };

    loadApiPromoConfig();

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="api-mode-page">
      <div className="api-console-grid">
        {apiPromoEnabled && (
          <div className={`api-promo-shell ${apiPromoBarOpen ? '' : 'minimized'}`}>
            {apiPromoBarOpen ? (
              <>
                <div className="api-promo-banner">
                  <button
                    type="button"
                    className="api-promo-link"
                    aria-label="打开 GPT Pool 网站"
                    title="打开 GPT Pool 网站"
                    onClick={onOpenGptPool}
                  >
                    <span className="api-promo-ad-label">广告</span>
                    <span className="api-promo-brand">GPT Pool</span>
                    <span className="api-promo-title">公益站点，注册免费获取10$额度</span>
                  </button>
                  <button
                    type="button"
                    className="api-promo-action"
                    onClick={onConfigureGptPoolApi}
                  >
                    自动配置
                  </button>
                </div>
                <button
                  type="button"
                  className="api-promo-close"
                  aria-label="关闭广告"
                  title="关闭广告"
                  onClick={() => onSetApiPromoBarOpen(false)}
                >
                  ×
                </button>
              </>
            ) : (
              <button
                type="button"
                className="api-promo-mini-window"
                aria-label="展开公益站点广告"
                title="展开公益站点广告"
                onClick={() => onSetApiPromoBarOpen(true)}
              >
                <span className="api-promo-mini-label">广告</span>
              </button>
            )}
          </div>
        )}

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
                        </div>
                        <div className="account-badges account-card-badges">
                          <span className="plan-badge plan-api">API</span>
                          <span className={`status-badge ${configured ? 'api-status-ready' : 'auth-error'}`}>
                            {configured ? '已配置' : '未完整'}
                          </span>
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
                          <div className="api-profile-card-info-row">
                            <span className="api-profile-card-label">API Key</span>
                            <span className={`api-profile-card-value ${profile.api_key ? '' : 'muted'}`}>
                              {profile.api_key ? '已保存' : '未配置'}
                            </span>
                          </div>
                        </div>
                      </div>

                      <div className="account-card-footer">
                        <div className="action-btns">
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
                              <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="m13 3-8 10h6l-1 8 9-11h-6l0-7Z" />
                              </svg>
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
                {totalPages > 0 && (
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
