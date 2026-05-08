import ConfirmDialog from './ConfirmDialog';

function getUpdateStatusText(updateModal) {
    if (updateModal.status === 'downloaded') return '已下载完成，重启后安装';
    if (updateModal.status === 'downloading') return `下载中 ${Math.round(updateModal.progress || 0)}%`;
    if (updateModal.status === 'error') return '更新失败';
    return '可下载';
}

function flushList(blocks, list) {
    if (list.length > 0) {
        blocks.push({ type: 'list', items: [...list] });
        list.length = 0;
    }
}

function parseUpdateNotes(notes) {
    const blocks = [];
    const list = [];
    const lines = String(notes || '').replace(/\r\n/g, '\n').split('\n');

    lines.forEach(rawLine => {
        const line = rawLine.trim();
        if (!line) {
            flushList(blocks, list);
            return;
        }

        const heading = line.match(/^#{1,6}\s+(.+)$/);
        if (heading) {
            flushList(blocks, list);
            const text = heading[1].trim();
            if (!['更新内容', '更新说明'].includes(text)) {
                blocks.push({ type: 'heading', text });
            }
            return;
        }

        const bullet = line.match(/^[-*]\s+(.+)$/);
        if (bullet) {
            list.push(bullet[1].trim());
            return;
        }

        flushList(blocks, list);
        blocks.push({ type: 'paragraph', text: line });
    });

    flushList(blocks, list);
    return blocks;
}

function UpdateNotes({ notes }) {
    const blocks = parseUpdateNotes(notes);
    if (blocks.length === 0) return null;

    return (
        <div className="update-dialog-notes-body">
            {blocks.map((block, index) => {
                if (block.type === 'heading') {
                    return <div className="update-dialog-notes-heading" key={`${block.type}-${index}`}>{block.text}</div>;
                }
                if (block.type === 'list') {
                    return (
                        <ul className="update-dialog-notes-list" key={`${block.type}-${index}`}>
                            {block.items.map((item, itemIndex) => (
                                <li key={`${index}-${itemIndex}`}>{item}</li>
                            ))}
                        </ul>
                    );
                }
                return <p className="update-dialog-notes-paragraph" key={`${block.type}-${index}`}>{block.text}</p>;
            })}
        </div>
    );
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
                            <div className="update-dialog-notes">
                                <div className="update-dialog-section-title">更新说明</div>
                                <UpdateNotes notes={updateModal.notes} />
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
