import ConfirmDialog from './ConfirmDialog';
import { useI18n } from '../i18n';

function getUpdateStatusText(updateModal, t) {
    if (updateModal.status === 'downloaded') return t('已下载完成，重启后安装');
    if (updateModal.status === 'downloading') return t('下载中 {progress}%', { progress: Math.round(updateModal.progress || 0) });
    if (updateModal.status === 'error') return t('更新失败');
    return t('可下载');
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
    const { language, t, translateRuntimeText } = useI18n();
    const progress = Math.max(0, Math.min(100, updateModal.progress || 0));

    return (
        <ConfirmDialog
            title={t('发现新版本 {version}', { version: updateModal.remoteVersion })}
            width="460px"
            content={(
                <div className="update-dialog-content">
                    <div className="update-dialog-headline">
                        {t('当前 {version}', { version: '' }).trim()} <strong>{updateModal.currentVersion || '--'}</strong>
                        <span className="update-dialog-sep">→</span>
                        {t('最新 {version}', { version: '' }).trim()} <strong>{updateModal.remoteVersion || '--'}</strong>
                    </div>
                    {updateModal.publishedAt && (
                        <div className="update-dialog-published">
                            {t('发布时间：{time}', { time: new Date(updateModal.publishedAt).toLocaleString(language) })}
                        </div>
                    )}
                    <div className="update-dialog-card">
                        <div className="update-dialog-row">
                            <span className="update-dialog-label">{t('更新状态')}</span>
                            <span className="update-dialog-value">{getUpdateStatusText(updateModal, t)}</span>
                        </div>
                        {updateModal.status === 'downloading' && (
                            <div
                                className="update-dialog-progress"
                                role="progressbar"
                                aria-label={t('下载更新')}
                                aria-valuemin={0}
                                aria-valuemax={100}
                                aria-valuenow={Math.round(progress)}
                            >
                                <div style={{ width: `${progress}%` }} />
                            </div>
                        )}
                        {updateModal.error && (
                            <div className="update-dialog-error">{translateRuntimeText(updateModal.error)}</div>
                        )}
                        {updateModal.notes && (
                            <div className="update-dialog-notes">
                                <div className="update-dialog-section-title">{t('更新说明')}</div>
                                <UpdateNotes notes={updateModal.notes} />
                            </div>
                        )}
                    </div>
                    <div className="update-dialog-tip">{t('下载完成后点击“重启安装”，应用会自动退出并安装新版本。')}</div>
                </div>
            )}
            isLoading={updateModal.loading}
            confirmText={updateModal.status === 'downloaded' ? t('重启安装') : t('下载更新')}
            loadingText={updateModal.status === 'downloading' ? t('下载中 {progress}%', { progress: Math.round(updateModal.progress || 0) }) : t('处理中...')}
            cancelText={t('稍后')}
            onConfirm={onConfirm}
            onCancel={onCancel}
        />
    );
}
