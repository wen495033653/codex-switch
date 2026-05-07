import ConfirmDialog from './ConfirmDialog';

function getUpdateStatusText(updateModal) {
    if (updateModal.status === 'downloaded') return '已下载完成，重启后安装';
    if (updateModal.status === 'downloading') return `下载中 ${Math.round(updateModal.progress || 0)}%`;
    if (updateModal.status === 'error') return '更新失败';
    return '可下载';
}

export default function UpdateDialog({ updateModal, onConfirm, onCancel }) {
    const progress = Math.max(0, Math.min(100, updateModal.progress || 0));

    return (
        <ConfirmDialog
            title={`发现新版本 ${updateModal.remoteVersion}`}
            width="460px"
            content={(
                <div className="update-dialog-content">
                    <div className="update-dialog-headline">
                        当前 <strong>{updateModal.currentVersion || '--'}</strong>
                        <span className="update-dialog-sep">→</span>
                        最新 <strong>{updateModal.remoteVersion || '--'}</strong>
                    </div>
                    {updateModal.publishedAt && (
                        <div className="update-dialog-published">
                            发布时间：{new Date(updateModal.publishedAt).toLocaleString()}
                        </div>
                    )}
                    <div className="update-dialog-card">
                        <div className="update-dialog-row">
                            <span className="update-dialog-label">更新状态</span>
                            <span className="update-dialog-value">{getUpdateStatusText(updateModal)}</span>
                        </div>
                        {updateModal.status === 'downloading' && (
                            <div className="update-dialog-progress">
                                <div style={{ width: `${progress}%` }} />
                            </div>
                        )}
                        {updateModal.error && (
                            <div className="update-dialog-error">{updateModal.error}</div>
                        )}
                        {updateModal.notes && (
                            <div className="update-dialog-row">
                                <span className="update-dialog-label">更新说明</span>
                                <span className="update-dialog-value">{updateModal.notes}</span>
                            </div>
                        )}
                    </div>
                    <div className="update-dialog-tip">下载完成后点击“重启安装”，应用会自动退出并安装新版本。</div>
                </div>
            )}
            isLoading={updateModal.loading}
            confirmText={updateModal.status === 'downloaded' ? '重启安装' : '下载更新'}
            loadingText={updateModal.status === 'downloading' ? `下载中 ${Math.round(updateModal.progress || 0)}%` : '处理中...'}
            cancelText="稍后"
            onConfirm={onConfirm}
            onCancel={onCancel}
        />
    );
}
