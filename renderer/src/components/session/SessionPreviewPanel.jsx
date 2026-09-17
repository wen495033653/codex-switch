import { useI18n } from '../../i18n';
import {
  formatSize,
  formatTime,
  previewMessageKey,
  statusLabel,
} from '../../utils/sessionManager';

export default function SessionPreviewPanel({
  activeConversation,
  activeCwd,
  activeSourcePath,
  loadEarlierMessages,
  loadPreview,
  messages,
  preview,
  previewEarlierLoading,
  previewItemRef,
  previewLoading,
  previewRef,
  previewTrimmedNewer,
}) {
  const { language, t, translateRuntimeText } = useI18n();
  return (
    <div className="session-preview-panel">
      {!activeConversation && (
        <div className="empty-state session-preview-empty">{t('选择一条会话查看预览')}</div>
      )}
      {activeConversation && (
        <>
          <div className="session-preview-head">
            <div>
              <h2 title={activeConversation.title}>{activeConversation.title}</h2>
              <p title={activeConversation.id}>{activeConversation.id}</p>
            </div>
            <span className={`session-status-pill ${activeConversation.status}`}>
              {statusLabel(activeConversation.status, t)}
            </span>
          </div>
          <div className="session-preview-meta">
            <span>
              {activeConversation.status === 'deleted'
                ? t('删除时间：{time}', { time: formatTime(activeConversation.updated_at, language, t) })
                : t('更新时间：{time}', { time: formatTime(activeConversation.updated_at, language, t) })}
            </span>
            <span>{t('大小：{size}', { size: formatSize(activeConversation.size_bytes) })}</span>
            <span title={activeCwd}>{t('工作目录：{path}', { path: activeCwd || t('未知') })}</span>
            <span title={activeSourcePath}>{t('路径：{path}', { path: activeConversation.relative_path })}</span>
          </div>
          {previewLoading && <div className="session-preview-loading">{t('读取中...')}</div>}
          {(activeConversation.parse_error || preview.parse_error) && (
            <div className="session-preview-error">
              {translateRuntimeText(activeConversation.parse_error || preview.parse_error)}
            </div>
          )}
          <div className="session-message-list" ref={previewRef}>
            {preview?.message_page?.has_more && (
              <div className="session-preview-page-control">
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={loadEarlierMessages}
                  disabled={previewEarlierLoading}
                >
                  {previewEarlierLoading ? t('加载中...') : t('加载更早内容')}
                </button>
              </div>
            )}
            {messages.map((message, index) => (
              <div
                key={previewMessageKey(message, index)}
                data-message-offset={message.offset ?? undefined}
                className={`session-message-row ${message.role === 'user' ? 'user' : 'assistant'}`}
              >
                <div className="session-message-meta">
                  {message.role === 'user' ? t('你') : 'Codex'} · {formatTime(message.timestamp, language, t)}
                </div>
                <div className="session-message-bubble">
                  {message.text}
                </div>
              </div>
            ))}
            {previewTrimmedNewer && (
              <div className="session-preview-latest-control">
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => loadPreview(previewItemRef.current)}
                  disabled={previewLoading}
                >
                  {t('回到最新内容')}
                </button>
              </div>
            )}
            {messages.length === 0 && !previewLoading && !previewEarlierLoading && (
              <div className="empty-state session-empty">{t('没有解析到可读对话')}</div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
