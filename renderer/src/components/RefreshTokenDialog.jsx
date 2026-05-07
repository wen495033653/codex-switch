import Modal from './Modal';

export default function RefreshTokenDialog({
    accountName,
    modal,
    onClose,
    onCopy,
    onRefresh
}) {
    return (
        <Modal title="查看 Refresh Token" onClose={onClose} width="560px">
            <div className="token-modal">
                <div className="token-modal-meta">
                    <div className="token-modal-label">账号</div>
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
                    <div className="token-modal-error">{modal.error}</div>
                )}
                <div className="token-modal-actions">
                    <button className="btn btn-secondary" onClick={onClose} disabled={modal.loading}>
                        关闭
                    </button>
                    <button className="btn btn-secondary" onClick={onRefresh} disabled={modal.loading}>
                        {modal.loading ? '刷新中...' : '刷新 Refresh Token'}
                    </button>
                    <button className="btn btn-primary" onClick={onCopy} disabled={modal.loading}>
                        复制 Refresh Token
                    </button>
                </div>
            </div>
        </Modal>
    );
}
