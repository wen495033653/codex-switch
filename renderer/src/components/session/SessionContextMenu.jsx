import { useI18n } from '../../i18n';

export default function SessionContextMenu({
  contextMenu,
  handleDeleteSessions,
  handleExport,
  handlePurgeDeleted,
  handleRestoreDeleted,
  handleSetStatus,
  setContextMenu,
}) {
  const { t } = useI18n();
  return (
    <div
      className="session-context-menu"
      role="menu"
      style={{ left: contextMenu.x, top: contextMenu.y }}
      onClick={event => event.stopPropagation()}
    >
      {contextMenu.item.status === 'deleted' ? (
        <>
          <button type="button" role="menuitem" onClick={() => {
            const item = contextMenu.item;
            setContextMenu(null);
            handleRestoreDeleted([item.delete_id]);
          }}>{t('恢复')}</button>
          <button type="button" role="menuitem" className="danger" onClick={() => {
            const item = contextMenu.item;
            setContextMenu(null);
            handlePurgeDeleted([item.delete_id]);
          }}>{t('彻底删除')}</button>
        </>
      ) : (
        <>
          {contextMenu.item.status === 'active' && (
            <button type="button" role="menuitem" onClick={() => {
              const item = contextMenu.item;
              setContextMenu(null);
              handleSetStatus([item.relative_path], 'archived');
            }}>{t('归档')}</button>
          )}
          {contextMenu.item.status === 'archived' && (
            <button type="button" role="menuitem" onClick={() => {
              const item = contextMenu.item;
              setContextMenu(null);
              handleSetStatus([item.relative_path], 'active');
            }}>{t('取消归档')}</button>
          )}
          <button type="button" role="menuitem" onClick={() => {
            const item = contextMenu.item;
            setContextMenu(null);
            handleExport([item.relative_path]);
          }}>{t('导出')}</button>
          <button type="button" role="menuitem" className="danger" onClick={() => {
            const item = contextMenu.item;
            setContextMenu(null);
            handleDeleteSessions([item.relative_path]);
          }}>{t('删除')}</button>
        </>
      )}
    </div>
  );
}
