export default function AboutSettingsTab({
    appVersion,
    checkingUpdate,
    handleCheckUpdate,
    openRepository
}) {
    return (
        <section className="settings-section">
            <div className="settings-about-card-grid">
                <div className="settings-about-card">
                    <span className="settings-about-card-icon author" aria-hidden="true">
                        <svg viewBox="0 0 24 24" role="img">
                            <path d="M12 12.4a4.2 4.2 0 1 0 0-8.4 4.2 4.2 0 0 0 0 8.4Zm0 2.1c-3.95 0-7.2 2.43-7.2 5.42 0 .6.49 1.08 1.08 1.08h12.24c.59 0 1.08-.48 1.08-1.08 0-2.99-3.25-5.42-7.2-5.42Z" />
                        </svg>
                    </span>
                    <span className="settings-about-card-kicker">作者</span>
                    <span className="settings-about-card-title">会飞的蛋蛋面</span>
                </div>

                <button type="button" className="settings-about-card" onClick={openRepository}>
                    <span className="settings-about-card-icon github" aria-hidden="true">
                        <svg viewBox="0 0 24 24" role="img">
                            <path d="M12 2C6.48 2 2 6.58 2 12.24c0 4.52 2.87 8.35 6.85 9.71.5.1.68-.22.68-.49 0-.24-.01-.88-.01-1.73-2.79.62-3.38-1.38-3.38-1.38-.45-1.19-1.11-1.5-1.11-1.5-.91-.64.07-.63.07-.63 1 .07 1.53 1.06 1.53 1.06.9 1.57 2.35 1.12 2.92.86.09-.67.35-1.12.64-1.38-2.23-.26-4.57-1.14-4.57-5.07 0-1.12.39-2.03 1.03-2.75-.1-.26-.45-1.31.1-2.71 0 0 .84-.28 2.75 1.05A9.34 9.34 0 0 1 12 6.94c.85 0 1.7.12 2.5.34 1.91-1.33 2.75-1.05 2.75-1.05.55 1.4.2 2.45.1 2.71.64.72 1.03 1.63 1.03 2.75 0 3.94-2.35 4.8-4.58 5.06.36.32.68.95.68 1.91 0 1.38-.01 2.49-.01 2.83 0 .27.18.59.69.49A10.22 10.22 0 0 0 22 12.24C22 6.58 17.52 2 12 2Z" />
                        </svg>
                    </span>
                    <span className="settings-about-card-kicker">开源地址</span>
                    <span className="settings-about-card-title">查看代码 <span aria-hidden="true">↗</span></span>
                </button>

                <div className="settings-about-card">
                    <span className="settings-about-card-icon support" aria-hidden="true">
                        <svg viewBox="0 0 24 24" role="img">
                            <path d="M12 21.35 10.55 20.03C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.08A6.01 6.01 0 0 1 16.5 3C19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35Z" />
                        </svg>
                    </span>
                    <span className="settings-about-card-kicker">赞助支持</span>
                    <span className="settings-about-card-title">支持作者</span>
                </div>
            </div>

            <div className="settings-about-list">
                <div className="settings-about-row">
                    <span>应用名称</span>
                    <strong>Codex Switch</strong>
                </div>
                <div className="settings-about-row">
                    <span>当前版本</span>
                    <span className="settings-about-version-actions">
                        <strong>{appVersion || '--'}</strong>
                        <button className="btn btn-secondary" onClick={() => handleCheckUpdate(true)} disabled={checkingUpdate}>
                            {checkingUpdate ? '检查中...' : '检查更新'}
                        </button>
                    </span>
                </div>
            </div>
        </section>
    );
}
