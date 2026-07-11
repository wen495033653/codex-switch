import Modal from './Modal';
import { useI18n } from '../i18n';

export default function AddAccountModal({
    oauth,
    oauthCallbackSubmitting,
    oauthCallbackUrl,
    oauthTimeoutHint,
    refreshTokenInput,
    refreshTokenLoading,
    showRefreshTokenPanel,
    onCancelOauth,
    onCaptureCurrent,
    onClose,
    onCopyOauthUrl,
    onImportAccountsFromBackup,
    onImportByRefreshToken,
    onOauthCallbackUrlChange,
    onRefreshTokenInputChange,
    onStartOauth,
    onSubmitOauthCallbackUrl,
    onToggleRefreshTokenPanel
}) {
    const { t, translateRuntimeText } = useI18n();
    return (
        <Modal title={t('连接新账号')} onClose={onClose}>
            <div className="connect-modal">
                <section className="connect-block">
                    <button type="button" className="btn btn-primary connect-oauth-btn" onClick={onStartOauth} disabled={oauth.running}>
                        {oauth.running && <span className="oauth-spinner" aria-hidden="true" />}
                        <span>{oauth.running ? t('等待浏览器授权...') : t('✨ OAuth 自动登录 (推荐)')}</span>
                    </button>
                    {oauth.running && (
                        <div className="oauth-url-card">
                            <div className="oauth-wait-row">
                                <span className="oauth-spinner oauth-card-spinner" aria-hidden="true" />
                                <span className="oauth-wait-text">{t('正在等待网页登录完成')}</span>
                            </div>
                            {oauth.url && <div className="oauth-url-text">{oauth.url}</div>}
                            <div className="oauth-action-row">
                                {oauth.url && (
                                    <button type="button" className="btn btn-secondary oauth-copy-btn" onClick={onCopyOauthUrl}>
                                        {t('点击复制链接')}
                                    </button>
                                )}
                                <button type="button" className="btn btn-secondary oauth-cancel-btn" onClick={onCancelOauth}>
                                    {t('取消登录')}
                                </button>
                            </div>
                            <div className="oauth-hint-text">{translateRuntimeText(oauthTimeoutHint)}</div>
                            <form
                                className="oauth-callback-form"
                                onSubmit={event => {
                                    event.preventDefault();
                                    onSubmitOauthCallbackUrl();
                                }}
                            >
                                <label className="oauth-callback-label" htmlFor="oauth-callback-url">
                                    {t('回调 URL')}
                                </label>
                                <div className="oauth-callback-row">
                                    <input
                                        id="oauth-callback-url"
                                        className="search-input oauth-callback-input"
                                        placeholder="http://localhost:1455/auth/callback?code=...&state=..."
                                        value={oauthCallbackUrl}
                                        onChange={event => onOauthCallbackUrlChange(event.target.value)}
                                        disabled={oauthCallbackSubmitting}
                                    />
                                    <button
                                        className="btn btn-secondary oauth-callback-submit"
                                        type="submit"
                                        disabled={oauthCallbackSubmitting || !oauthCallbackUrl.trim()}
                                    >
                                        {oauthCallbackSubmitting ? t('提交中...') : t('提交回调 URL')}
                                    </button>
                                </div>
                                <div className="oauth-hint-text">
                                    {t('远程浏览器模式：授权跳转到 localhost 回调页后，复制完整 URL 到这里。')}
                                </div>
                            </form>
                        </div>
                    )}
                    {oauth.error && <div className="oauth-error-text">{translateRuntimeText(oauth.error)}</div>}
                    {oauth.errorCode && <div className="oauth-error-code">Error Code: {oauth.errorCode}</div>}
                </section>

                <section className="connect-block connect-refresh-compact">
                        <button
                            type="button"
                            className={`btn btn-secondary connect-refresh-toggle ${showRefreshTokenPanel ? 'open' : ''}`}
                        onClick={onToggleRefreshTokenPanel}
                    >
                        <span>{t('Refresh Token 导入')}</span>
                        <span className="connect-refresh-arrow">{showRefreshTokenPanel ? '▴' : '▾'}</span>
                    </button>
                    {showRefreshTokenPanel && (
                        <div className="connect-refresh-panel">
                            <textarea
                                className="search-input connect-refresh-input"
                                placeholder={t('粘贴 refresh_token...')}
                                value={refreshTokenInput}
                                onChange={e => onRefreshTokenInputChange(e.target.value)}
                            />
                            <button type="button" className="btn btn-secondary connect-refresh-submit" onClick={onImportByRefreshToken} disabled={refreshTokenLoading}>
                                {refreshTokenLoading ? t('导入中...') : t('导入账号')}
                            </button>
                        </div>
                    )}
                </section>

                <section className="connect-block">
                    <div className="connect-block-head">
                        <div className="connect-block-title">{t('本地导入')}</div>
                        <div className="connect-block-desc">{t('从当前设备读取账号配置或导入备份文件')}</div>
                    </div>
                    <div className="connect-inline-actions">
                        <button type="button" className="btn btn-secondary connect-inline-btn" onClick={onCaptureCurrent}>
                            {t('📂 读取本机 auth.json')}
                        </button>
                        <button type="button" className="btn btn-secondary connect-inline-btn" onClick={onImportAccountsFromBackup}>
                            {t('📥 导入 JSON 备份')}
                        </button>
                    </div>
                </section>
            </div>
        </Modal>
    );
}
