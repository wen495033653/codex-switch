import { useState } from 'react';

const SUPPORT_QR_CODES = [
    {
        label: '支付宝',
        src: `${import.meta.env.BASE_URL}assets/support/alipay-qr.png`
    },
    {
        label: '微信支付',
        src: `${import.meta.env.BASE_URL}assets/support/wechat-qr.png`
    }
];

export default function AboutSettingsTab({
    appVersion,
    checkingUpdate,
    handleCheckUpdate,
    onOpenGptPool,
    openRepository
}) {
    const [supportVisible, setSupportVisible] = useState(false);

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

                <button type="button" className="settings-about-card" onClick={onOpenGptPool}>
                    <span className="settings-about-card-icon relay" aria-hidden="true">
                        <svg viewBox="0 0 24 24" role="img">
                            <path d="M4.75 6.5A2.75 2.75 0 0 1 7.5 3.75h9A2.75 2.75 0 0 1 19.25 6.5v11A2.75 2.75 0 0 1 16.5 20.25h-9A2.75 2.75 0 0 1 4.75 17.5v-11Zm2.75-1.25c-.69 0-1.25.56-1.25 1.25v11c0 .69.56 1.25 1.25 1.25h9c.69 0 1.25-.56 1.25-1.25v-11c0-.69-.56-1.25-1.25-1.25h-9Zm1.75 3.5a.75.75 0 0 1 .75-.75h4a.75.75 0 0 1 0 1.5h-4a.75.75 0 0 1-.75-.75Zm0 3.25a.75.75 0 0 1 .75-.75h4a.75.75 0 0 1 0 1.5h-4a.75.75 0 0 1-.75-.75Zm0 3.25a.75.75 0 0 1 .75-.75h2.4a.75.75 0 0 1 0 1.5H10a.75.75 0 0 1-.75-.75Z" />
                        </svg>
                    </span>
                    <span className="settings-about-card-kicker">中转站</span>
                    <span className="settings-about-card-title">访问站点</span>
                </button>

                <button type="button" className="settings-about-card" onClick={() => setSupportVisible(true)}>
                    <span className="settings-about-card-icon support" aria-hidden="true">
                        <svg viewBox="0 0 24 24" role="img">
                            <path d="M12 21.35 10.55 20.03C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.08A6.01 6.01 0 0 1 16.5 3C19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35Z" />
                        </svg>
                    </span>
                    <span className="settings-about-card-kicker">赞助支持</span>
                    <span className="settings-about-card-title">支持作者</span>
                </button>
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

            {supportVisible && (
                <SupportDialog onClose={() => setSupportVisible(false)} />
            )}
        </section>
    );
}

function SupportDialog({ onClose }) {
    return (
        <div className="modal-overlay" onClick={e => e.target === e.currentTarget && onClose()}>
            <div className="modal-content support-dialog" role="dialog" aria-modal="true" aria-labelledby="support-dialog-title">
                <div className="support-dialog-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24" role="img">
                        <path d="M4 4.5h11.5v7.25A5.75 5.75 0 0 1 9.75 17.5h-.5A5.75 5.75 0 0 1 3.5 11.75V5A.5.5 0 0 1 4 4.5Zm13 2h1.25a2.75 2.75 0 0 1 0 5.5H17V6.5Zm0 2V10h1.25a.75.75 0 0 0 0-1.5H17ZM6.5 2.5a.75.75 0 0 1 1.5 0v.75a.75.75 0 0 1-1.5 0V2.5Zm4 0a.75.75 0 0 1 1.5 0v.75a.75.75 0 0 1-1.5 0V2.5ZM5 20a1 1 0 0 1 1-1h9a1 1 0 1 1 0 2H6a1 1 0 0 1-1-1Z" />
                    </svg>
                </div>
                <h3 className="support-dialog-title" id="support-dialog-title">赞助支持</h3>
                <p className="support-dialog-desc">
                    如果您觉得本工具对您有帮助，欢迎扫码请作者喝杯咖啡！您的支持是我持续维护项目的最大动力。
                </p>

                <div className="support-qr-grid">
                    {SUPPORT_QR_CODES.map(item => (
                        <div className="support-qr-card" key={item.label}>
                            <div className="support-qr-frame">
                                <img src={item.src} alt={`${item.label}收款二维码`} decoding="async" />
                            </div>
                            <strong>{item.label}</strong>
                        </div>
                    ))}
                </div>

                <button type="button" className="btn btn-secondary support-dialog-close" onClick={onClose}>
                    关闭
                </button>
            </div>
        </div>
    );
}
