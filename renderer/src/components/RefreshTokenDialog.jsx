import Modal from './Modal';
import { useI18n } from '../i18n';

export default function RefreshTokenDialog({
    accountName,
    modal,
    onClose,
    onCopy,
    onRefresh
}) {
    const { t, translateRuntimeText } = useI18n();
    return (
        <Modal title={t('查看 Refresh Token')} onClose={onClose} width="560px">
            <div className="token-modal">
                <div className="token-modal-meta">
                    <div className="token-modal-label">{t('账号')}</div>
                    <div className="token-modal-name" title={accountName}>
                        {accountName || '--'}
                    </div>
                </div>
                <textarea
                    className="token-modal-text"
                    readOnly
                    value={modal.refreshToken}
                />
                {modal.error && (
                    <div className="token-modal-error">{translateRuntimeText(modal.error)}</div>
                )}
                <div className="token-modal-actions">
                    <button type="button" className="btn btn-secondary" onClick={onClose} disabled={modal.loading}>
                        {t('关闭')}
                    </button>
                    <button type="button" className="btn btn-secondary" onClick={onRefresh} disabled={modal.loading}>
                        {modal.loading ? t('刷新中...') : t('刷新 Refresh Token')}
                    </button>
                    <button type="button" className="btn btn-primary" onClick={onCopy} disabled={modal.loading}>
                        {t('复制 Refresh Token')}
                    </button>
                </div>
            </div>
        </Modal>
    );
}
