import { useI18n } from '../../i18n';

export default function SessionContextMenu({
  contextMenu,
  handleDeleteSessions,
  handleExport,
  handleSetStatus,
  setContextMenu,
}) {
  const { t } = useI18n();
  const { item, x, y } = contextMenu;
  const paths = [item.relative_path];
  const selectAction = action => {
    setContextMenu(null);
    action();
  };
  return (
    <div
      className="session-context-menu"
      role="menu"
      style={{ left: x, top: y }}
      onClick={event => event.stopPropagation()}
    >
      {item.status === 'active' && (
        <button type="button" role="menuitem" onClick={() => selectAction(() => handleSetStatus(paths, 'archived'))}>
          {t('归档')}
        </button>
      )}
      {item.status === 'archived' && (
        <button type="button" role="menuitem" onClick={() => selectAction(() => handleSetStatus(paths, 'active'))}>
          {t('取消归档')}
        </button>
      )}
      <button type="button" role="menuitem" onClick={() => selectAction(() => handleExport(paths))}>
        {t('导出')}
      </button>
      <button type="button" role="menuitem" className="danger" onClick={() => selectAction(() => handleDeleteSessions(paths))}>
        {t('永久删除')}
      </button>
    </div>
  );
}
