import { useEffect, useMemo, useRef, useState } from 'react';
import ConfirmDialog from './ConfirmDialog';

const STATUS_FILTERS = [
  { key: 'all', label: '全部' },
  { key: 'active', label: '进行中' },
  { key: 'archived', label: '已归档' },
  { key: 'deleted', label: '已删除' }
];

const PAGE_SIZE_OPTIONS = [50, 100, 200];

function formatSize(bytes) {
  const value = Number(bytes) || 0;
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${value} B`;
}

function formatTime(value) {
  if (!value) return '未知';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function statusLabel(status) {
  if (status === 'archived') return '已归档';
  if (status === 'deleted') return '已删除';
  return '进行中';
}

function lower(value) {
  return String(value || '').toLowerCase();
}

function deletedActiveKey(item) {
  return `deleted:${item.delete_id}`;
}

function deletedPreviewConversation(item) {
  return {
    id: item.id,
    title: item.title,
    status: 'deleted',
    updated_at: item.deleted_at,
    size_bytes: item.size_bytes,
    cwd: item.cwd,
    source_path: item.root_path,
    relative_path: item.original_relative_path
  };
}

export default function SessionManagerPage({ toast, toastError }) {
  const [rootPath, setRootPath] = useState('');
  const [conversations, setConversations] = useState([]);
  const [deletedSessions, setDeletedSessions] = useState([]);
  const [search, setSearch] = useState('');
  const [statusFilter, setStatusFilter] = useState('all');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [selected, setSelected] = useState(() => new Set());
  const [selectedDeleted, setSelectedDeleted] = useState(() => new Set());
  const [activePath, setActivePath] = useState('');
  const [preview, setPreview] = useState(null);
  const [contextMenu, setContextMenu] = useState(null);
  const [purgeConfirm, setPurgeConfirm] = useState(null);
  const [conflictConfirm, setConflictConfirm] = useState(null);
  const [loading, setLoading] = useState(false);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [actionLoading, setActionLoading] = useState(false);
  const [previewWidth, setPreviewWidth] = useState(0);
  const previewRef = useRef(null);
  const previewRequestRef = useRef(0);

  const isDeletedView = statusFilter === 'deleted';

  const filteredConversations = useMemo(() => {
    if (isDeletedView) return [];
    const query = lower(search.trim());
    return conversations.filter(item => {
      if (statusFilter !== 'all' && item.status !== statusFilter) return false;
      if (!query) return true;
      return lower(item.title).includes(query)
        || lower(item.id).includes(query)
        || lower(item.cwd).includes(query)
        || lower(item.relative_path).includes(query);
    });
  }, [conversations, isDeletedView, search, statusFilter]);

  const filteredDeletedSessions = useMemo(() => {
    if (!isDeletedView) return [];
    const query = lower(search.trim());
    return deletedSessions.filter(item => {
      if (!query) return true;
      return lower(item.title).includes(query)
        || lower(item.id).includes(query)
        || lower(item.cwd).includes(query)
        || lower(item.original_relative_path).includes(query)
        || lower(item.root_path).includes(query);
    });
  }, [deletedSessions, isDeletedView, search]);

  const visibleItems = isDeletedView ? filteredDeletedSessions : filteredConversations;
  const totalPages = Math.max(1, Math.ceil(visibleItems.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const pageStart = (currentPage - 1) * pageSize;
  const pageItems = visibleItems.slice(pageStart, pageStart + pageSize);
  const selectedPaths = useMemo(() => Array.from(selected), [selected]);
  const selectedDeletedIds = useMemo(() => Array.from(selectedDeleted), [selectedDeleted]);

  const selectedItems = useMemo(() => {
    const selectedSet = new Set(selectedPaths);
    return conversations.filter(item => selectedSet.has(item.relative_path));
  }, [conversations, selectedPaths]);

  const selectedDeletedItems = useMemo(() => {
    const selectedSet = new Set(selectedDeletedIds);
    return deletedSessions.filter(item => selectedSet.has(item.delete_id));
  }, [deletedSessions, selectedDeletedIds]);

  const selectedSize = (isDeletedView ? selectedDeletedItems : selectedItems)
    .reduce((sum, item) => sum + (Number(item.size_bytes) || 0), 0);
  const selectedCount = isDeletedView ? selectedDeletedIds.length : selectedPaths.length;
  const allPageSelected = pageItems.length > 0 && pageItems.every(item => (
    isDeletedView ? selectedDeleted.has(item.delete_id) : selected.has(item.relative_path)
  ));

  const loadDeletedSessions = async () => {
    try {
      const res = await window.api.listDeletedSessions();
      const nextDeleted = Array.isArray(res.deleted)
        ? res.deleted.map(item => ({ ...item, status: 'deleted' }))
        : [];
      setDeletedSessions(nextDeleted);
      setSelectedDeleted(prev => {
        const existing = new Set(nextDeleted.map(item => item.delete_id));
        return new Set(Array.from(prev).filter(id => existing.has(id)));
      });
      return nextDeleted;
    } catch (err) {
      toastError(err, '读取已删除会话失败', 6000);
      return [];
    }
  };

  const loadPreview = async (item) => {
    if (!item) return;
    if (item.status === 'deleted') {
      const requestId = previewRequestRef.current + 1;
      previewRequestRef.current = requestId;
      setActivePath(deletedActiveKey(item));
      setPreviewLoading(true);
      try {
        const res = await window.api.previewDeletedSession(item.delete_id);
        if (previewRequestRef.current !== requestId) return;
        setPreview(res);
      } catch (err) {
        if (previewRequestRef.current !== requestId) return;
        setPreview({
          conversation: deletedPreviewConversation(item),
          messages: [],
          parse_error: err && err.message ? err.message : String(err || '读取已删除会话预览失败')
        });
        toastError(err, '读取已删除会话预览失败', 6000);
      } finally {
        if (previewRequestRef.current === requestId) setPreviewLoading(false);
      }
      return;
    }
    if (!rootPath) return;
    if (item.parse_error && Number(item.size_bytes) === 0) {
      setActivePath(item.relative_path);
      setPreview({
        conversation: item,
        messages: [],
        parse_error: item.parse_error
      });
      return;
    }
    const requestId = previewRequestRef.current + 1;
    previewRequestRef.current = requestId;
    setActivePath(item.relative_path);
    setPreviewLoading(true);
    try {
      const res = await window.api.previewSession({
        root: rootPath,
        relativePath: item.relative_path
      });
      if (previewRequestRef.current !== requestId) return;
      setPreview(res);
    } catch (err) {
      if (previewRequestRef.current !== requestId) return;
      setPreview(null);
      toastError(err, '读取会话预览失败', 6000);
    } finally {
      if (previewRequestRef.current === requestId) setPreviewLoading(false);
    }
  };

  const refreshSessions = async (nextRoot = rootPath) => {
    setLoading(true);
    let nextConversations = conversations;
    let nextDeleted = deletedSessions;
    try {
      const res = await window.api.scanSessions(nextRoot || undefined);
      nextConversations = Array.isArray(res.conversations) ? res.conversations : [];
      setRootPath(res.root || nextRoot || '');
      setConversations(nextConversations);
      setSelected(prev => {
        const existing = new Set(nextConversations.map(item => item.relative_path));
        return new Set(Array.from(prev).filter(path => existing.has(path)));
      });
    } catch (err) {
      toastError(err, '扫描会话失败', 7000);
    }
    nextDeleted = await loadDeletedSessions();
    if (activePath) {
      const activeExists = nextConversations.some(item => item.relative_path === activePath)
        || nextDeleted.some(item => deletedActiveKey(item) === activePath);
      if (!activeExists) {
        setActivePath('');
        setPreview(null);
      }
    }
    setLoading(false);
  };

  useEffect(() => {
    refreshSessions('');
  }, []);

  useEffect(() => {
    setPage(1);
  }, [search, statusFilter, pageSize]);

  useEffect(() => {
    if (page > totalPages) setPage(totalPages);
  }, [page, totalPages]);

  useEffect(() => {
    const node = previewRef.current;
    if (!node || typeof ResizeObserver === 'undefined') return undefined;
    const observer = new ResizeObserver(entries => {
      const width = entries[0] && entries[0].contentRect ? entries[0].contentRect.width : 0;
      setPreviewWidth(width);
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!contextMenu) return undefined;
    const close = () => setContextMenu(null);
    window.addEventListener('click', close);
    window.addEventListener('keydown', close);
    return () => {
      window.removeEventListener('click', close);
      window.removeEventListener('keydown', close);
    };
  }, [contextMenu]);

  const handleSelectRoot = async () => {
    try {
      const res = await window.api.selectSessionRoot();
      await refreshSessions(res.path || '');
    } catch (err) {
      toastError(err, '选择 Codex 数据目录失败', 6000);
    }
  };

  const toggleSelection = (item, checked) => {
    if (isDeletedView) {
      setSelectedDeleted(prev => {
        const next = new Set(prev);
        if (checked) next.add(item.delete_id);
        else next.delete(item.delete_id);
        return next;
      });
      return;
    }
    setSelected(prev => {
      const next = new Set(prev);
      if (checked) next.add(item.relative_path);
      else next.delete(item.relative_path);
      return next;
    });
  };

  const toggleSelectFiltered = () => {
    if (isDeletedView) {
      setSelectedDeleted(prev => {
        const next = new Set(prev);
        if (allPageSelected) {
          pageItems.forEach(item => next.delete(item.delete_id));
        } else {
          pageItems.forEach(item => next.add(item.delete_id));
        }
        return next;
      });
      return;
    }
    setSelected(prev => {
      const next = new Set(prev);
      if (allPageSelected) {
        pageItems.forEach(item => next.delete(item.relative_path));
      } else {
        pageItems.forEach(item => next.add(item.relative_path));
      }
      return next;
    });
  };

  const clearSelection = () => {
    if (isDeletedView) setSelectedDeleted(new Set());
    else setSelected(new Set());
  };

  const runAction = async (action, successMessage, errorMessage, refreshRoot = rootPath) => {
    if (actionLoading) return null;
    setActionLoading(true);
    try {
      const res = await action();
      if (res && res.report && res.report.conflict_action_required) return res;
      toast((res && res.message) || successMessage);
      await refreshSessions(refreshRoot);
      return res;
    } catch (err) {
      toastError(err, errorMessage || successMessage, 7000);
      return null;
    } finally {
      setActionLoading(false);
    }
  };

  const handleExport = (paths = selectedPaths) => {
    if (paths.length === 0) {
      toast('请先选择要导出的会话');
      return;
    }
    runAction(
      () => window.api.exportSessions({ root: rootPath, relativePaths: paths }),
      '导出会话完成',
      '导出会话失败'
    );
  };

  const handleImport = () => {
    runAction(() => window.api.importSessions(rootPath), '导入会话完成', '导入会话失败');
  };

  const openConflictDialog = ({ title, message, conflicts, onResolve }) => {
    setConflictConfirm({
      title,
      message,
      conflicts: Array.isArray(conflicts) ? conflicts : [],
      onResolve
    });
  };

  const resolveConflictDialog = (strategy) => {
    if (!conflictConfirm || actionLoading) return;
    const action = conflictConfirm.onResolve;
    setConflictConfirm(null);
    action(strategy);
  };

  const handleRestoreDeleted = (deleteIds = selectedDeletedIds, conflictStrategy = 'ask') => {
    const ids = Array.from(new Set(deleteIds));
    if (ids.length === 0) {
      toast('请先选择要恢复的会话');
      return;
    }
    const idSet = new Set(ids);
    const items = deletedSessions.filter(item => idSet.has(item.delete_id));
    const targetRoot = rootPath || (items[0] && items[0].root_path) || '';
    if (!targetRoot) {
      toast('请先选择 Codex 数据目录');
      return;
    }
    runAction(
      () => window.api.restoreDeletedSessions({ root: targetRoot, deleteIds: ids, conflictStrategy }),
      '恢复会话完成',
      '恢复会话失败',
      targetRoot
    ).then(res => {
      if (res && res.report && res.report.conflict_action_required) {
        openConflictDialog({
          title: '恢复会话存在冲突',
          message: '恢复目标位置已有会话文件，请选择这批冲突的处理方式。',
          conflicts: res.report.conflicts,
          onResolve: strategy => handleRestoreDeleted(ids, strategy)
        });
        return;
      }
      if (res) setSelectedDeleted(prev => new Set(Array.from(prev).filter(id => !ids.includes(id))));
    });
  };

  const handlePurgeDeleted = (deleteIds = selectedDeletedIds) => {
    const ids = Array.from(new Set(deleteIds));
    if (ids.length === 0) {
      toast('请先选择要彻底删除的会话');
      return;
    }
    const idSet = new Set(ids);
    const items = deletedSessions.filter(item => idSet.has(item.delete_id));
    setPurgeConfirm({
      deleteIds: ids,
      items,
      totalSize: items.reduce((sum, item) => sum + (Number(item.size_bytes) || 0), 0)
    });
  };

  const cancelPurgeDeleted = () => {
    if (!actionLoading) setPurgeConfirm(null);
  };

  const confirmPurgeDeleted = () => {
    if (!purgeConfirm || actionLoading) return;
    const ids = purgeConfirm.deleteIds;
    runAction(
      () => window.api.purgeDeletedSessions(ids),
      '彻底删除完成',
      '彻底删除失败'
    ).then(res => {
      if (res) setSelectedDeleted(prev => new Set(Array.from(prev).filter(id => !ids.includes(id))));
      setPurgeConfirm(null);
    });
  };

  const handleUpdateCwd = async (paths = selectedPaths) => {
    if (paths.length === 0) {
      toast('请先选择要修改工作目录的会话');
      return;
    }
    let cwd = '';
    try {
      const res = await window.api.selectSessionWorkdir();
      cwd = (res && res.path) || '';
    } catch (err) {
      toastError(err, '选择工作目录失败', 5000);
      return;
    }
    if (!cwd.trim()) return;
    runAction(
      () => window.api.updateSessionCwd({ root: rootPath, relativePaths: paths, cwd }),
      '修改工作目录完成',
      '修改工作目录失败'
    );
  };

  const openContextMenu = (event, item) => {
    event.preventDefault();
    event.stopPropagation();
    setContextMenu({
      x: event.clientX,
      y: event.clientY,
      item
    });
  };

  const activeConversation = preview && preview.conversation ? preview.conversation : null;
  const messages = preview && Array.isArray(preview.messages) ? preview.messages : [];
  const bubbleMaxWidth = previewWidth > 0
    ? Math.max(180, Math.min(Math.floor(previewWidth * 0.76), previewWidth - 28))
    : undefined;

  return (
    <div className="session-manager-page">
      <div className="session-toolbar">
        <div className="session-root">
          <span className="session-root-label">Codex 数据目录</span>
          <span className="session-root-path" title={rootPath}>{rootPath || '未识别'}</span>
        </div>
        <div className="session-actions">
          <button type="button" className="btn btn-secondary" onClick={handleSelectRoot} disabled={actionLoading}>
            选择目录
          </button>
          <button type="button" className="btn btn-secondary" onClick={() => refreshSessions(rootPath)} disabled={loading || actionLoading}>
            {loading ? '刷新中...' : '刷新'}
          </button>
          {isDeletedView ? (
            <>
              <button type="button" className="btn btn-secondary" onClick={() => handleRestoreDeleted()} disabled={selectedDeletedIds.length === 0 || actionLoading}>
                恢复
              </button>
              <button type="button" className="btn btn-danger" onClick={() => handlePurgeDeleted()} disabled={selectedDeletedIds.length === 0 || actionLoading}>
                彻底删除
              </button>
            </>
          ) : (
            <>
              <button type="button" className="btn btn-secondary" onClick={handleImport} disabled={!rootPath || actionLoading}>
                导入
              </button>
              <button type="button" className="btn btn-secondary" onClick={() => handleExport()} disabled={selectedPaths.length === 0 || actionLoading}>
                导出
              </button>
              <button type="button" className="btn btn-secondary" onClick={() => handleUpdateCwd()} disabled={selectedPaths.length === 0 || actionLoading}>
                修改工作目录
              </button>
            </>
          )}
        </div>
      </div>

      <div className="session-filterbar">
        <div className="search-wrapper session-search">
          <span className="search-icon">🔍</span>
          <input
            className="search-input"
            placeholder="搜索标题、ID、工作目录或路径..."
            value={search}
            onChange={event => setSearch(event.target.value)}
          />
        </div>
        <div className="nav-tabs session-status-tabs">
          {STATUS_FILTERS.map(item => {
            const count = item.key === 'all'
              ? conversations.length
              : item.key === 'deleted'
                ? deletedSessions.length
                : conversations.filter(conversation => conversation.status === item.key).length;
            return (
              <button
                key={item.key}
                type="button"
                className={`nav-item session-status-tab ${statusFilter === item.key ? 'active' : ''}`}
                onClick={() => setStatusFilter(item.key)}
              >
                {item.label} <span>{count}</span>
              </button>
            );
          })}
        </div>
        <button type="button" className="btn btn-secondary" onClick={clearSelection} disabled={selectedCount === 0}>
          清空选择
        </button>
      </div>

      <div className="session-workspace">
        <div className="session-list-panel">
          <div className="session-list-header">
            <label className="session-checkbox">
              <input
                type="checkbox"
                checked={allPageSelected}
                onChange={toggleSelectFiltered}
              />
            </label>
            <span>标题</span>
            <span>状态</span>
            <span>{isDeletedView ? '删除时间' : '更新时间'}</span>
            <span>大小</span>
          </div>
          <div className="session-list">
            {pageItems.map(item => {
              const rowKey = isDeletedView ? item.delete_id : item.relative_path;
              const activeKey = isDeletedView ? deletedActiveKey(item) : item.relative_path;
              const isSelected = isDeletedView ? selectedDeleted.has(item.delete_id) : selected.has(item.relative_path);
              return (
                <button
                  key={rowKey}
                  type="button"
                  className={`session-row ${activePath === activeKey ? 'active' : ''}`}
                  onClick={() => loadPreview(item)}
                  onContextMenu={event => openContextMenu(event, item)}
                >
                  <span className="session-checkbox" onClick={event => event.stopPropagation()}>
                    <input
                      type="checkbox"
                      checked={isSelected}
                      onChange={event => toggleSelection(item, event.target.checked)}
                    />
                  </span>
                  <span className="session-title-cell">
                    <strong title={item.title}>{item.title}</strong>
                    <small title={item.id}>{item.id}</small>
                    {item.cwd && <em title={item.cwd}>{item.cwd}</em>}
                  </span>
                  <span className={`session-status-pill ${item.status}`}>{statusLabel(item.status)}</span>
                  <span className="session-muted">{formatTime(isDeletedView ? item.deleted_at : item.updated_at)}</span>
                  <span className="session-muted">{formatSize(item.size_bytes)}</span>
                </button>
              );
            })}
            {visibleItems.length === 0 && (
              <div className="empty-state session-empty">{isDeletedView ? '暂无已删除会话' : '暂无会话数据'}</div>
            )}
          </div>
          <div className="session-footer">
            <span>总计 {isDeletedView ? deletedSessions.length : conversations.length} 个</span>
            <span>筛选 {visibleItems.length} 个</span>
            <span>本页 {pageItems.length} 个</span>
            <span>已选 {selectedCount} 个</span>
            <span>{formatSize(selectedSize)}</span>
            <div className="session-pagination">
              <button type="button" className="btn btn-secondary" onClick={() => setPage(1)} disabled={currentPage <= 1}>
                首页
              </button>
              <button type="button" className="btn btn-secondary" onClick={() => setPage(value => Math.max(1, value - 1))} disabled={currentPage <= 1}>
                上页
              </button>
              <span>{currentPage}/{totalPages}</span>
              <button type="button" className="btn btn-secondary" onClick={() => setPage(value => Math.min(totalPages, value + 1))} disabled={currentPage >= totalPages}>
                下页
              </button>
              <select value={pageSize} onChange={event => setPageSize(Number(event.target.value))}>
                {PAGE_SIZE_OPTIONS.map(value => (
                  <option key={value} value={value}>每页 {value}</option>
                ))}
              </select>
            </div>
          </div>
        </div>

        <div className="session-preview-panel">
          {!activeConversation && (
            <div className="empty-state session-preview-empty">选择一条会话查看预览</div>
          )}
          {activeConversation && (
            <>
              <div className="session-preview-head">
                <div>
                  <h2 title={activeConversation.title}>{activeConversation.title}</h2>
                  <p title={activeConversation.id}>{activeConversation.id}</p>
                </div>
                <span className={`session-status-pill ${activeConversation.status}`}>
                  {statusLabel(activeConversation.status)}
                </span>
              </div>
              <div className="session-preview-meta">
                <span>{activeConversation.status === 'deleted' ? '删除时间' : '更新时间'}：{formatTime(activeConversation.updated_at)}</span>
                <span>大小：{formatSize(activeConversation.size_bytes)}</span>
                <span title={activeConversation.cwd || ''}>工作目录：{activeConversation.cwd || '未知'}</span>
                <span title={activeConversation.source_path}>路径：{activeConversation.relative_path}</span>
              </div>
              {previewLoading && <div className="session-preview-loading">读取中...</div>}
              {(activeConversation.parse_error || preview.parse_error) && (
                <div className="session-preview-error">
                  {activeConversation.parse_error || preview.parse_error}
                </div>
              )}
              <div className="session-message-list" ref={previewRef}>
                {messages.map((message, index) => (
                  <div
                    key={`${message.role}-${index}`}
                    className={`session-message-row ${message.role === 'user' ? 'user' : 'assistant'}`}
                  >
                    <div className="session-message-meta">
                      {message.role === 'user' ? '你' : 'Codex'} · {formatTime(message.timestamp)}
                    </div>
                    <div className="session-message-bubble" style={bubbleMaxWidth ? { maxWidth: `${bubbleMaxWidth}px` } : undefined}>
                      {message.text}
                    </div>
                  </div>
                ))}
                {messages.length === 0 && !previewLoading && (
                  <div className="empty-state session-empty">没有解析到可读对话</div>
                )}
              </div>
            </>
          )}
        </div>
      </div>

      {contextMenu && (
        <div
          className="session-context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onClick={event => event.stopPropagation()}
        >
          {contextMenu.item.status === 'deleted' ? (
            <>
              <button type="button" onClick={() => {
                const item = contextMenu.item;
                setContextMenu(null);
                handleRestoreDeleted([item.delete_id]);
              }}>恢复</button>
              <button type="button" className="danger" onClick={() => {
                const item = contextMenu.item;
                setContextMenu(null);
                handlePurgeDeleted([item.delete_id]);
              }}>彻底删除</button>
            </>
          ) : (
            <>
              <button type="button" onClick={() => {
                const item = contextMenu.item;
                setContextMenu(null);
                handleExport([item.relative_path]);
              }}>导出</button>
              <button type="button" onClick={() => {
                const item = contextMenu.item;
                setContextMenu(null);
                handleUpdateCwd([item.relative_path]);
              }}>修改工作目录</button>
            </>
          )}
        </div>
      )}

      {conflictConfirm && (
        <div className="modal-overlay">
          <div className="modal-content modal-content-lg session-conflict-dialog">
            <h3>{conflictConfirm.title}</h3>
            <p>{conflictConfirm.message}</p>
            <div className="session-delete-list">
              {conflictConfirm.conflicts.slice(0, 8).map((item, index) => (
                <div key={`${item.target || item.relative_path || item.delete_id || index}`} className="session-delete-item">
                  <strong title={item.title || item.target || ''}>{item.title || item.target || '冲突会话'}</strong>
                  <span title={item.target || ''}>{item.target || item.relative_path || item.delete_id}</span>
                </div>
              ))}
              {conflictConfirm.conflicts.length > 8 && (
                <div className="session-delete-item">
                  <strong>还有 {conflictConfirm.conflicts.length - 8} 个冲突</strong>
                </div>
              )}
            </div>
            <div className="session-conflict-actions">
              <button type="button" className="btn btn-secondary" onClick={() => resolveConflictDialog('skip')} disabled={actionLoading}>
                跳过
              </button>
              <button type="button" className="btn btn-secondary" onClick={() => resolveConflictDialog('modify_id')} disabled={actionLoading}>
                修改 ID
              </button>
              <button type="button" className="btn btn-danger" onClick={() => resolveConflictDialog('overwrite')} disabled={actionLoading}>
                覆盖
              </button>
            </div>
          </div>
        </div>
      )}

      {purgeConfirm && (
        <ConfirmDialog
          title="确认彻底删除"
          width="460px"
          confirmText="彻底删除"
          loadingText="删除中..."
          confirmVariant="danger"
          isLoading={actionLoading}
          onCancel={cancelPurgeDeleted}
          onConfirm={confirmPurgeDeleted}
          content={(
            <div className="session-delete-confirm">
              <p>将从 Codex Switch 数据目录中彻底删除 {purgeConfirm.deleteIds.length} 个会话备份。</p>
              <p>彻底删除后无法恢复。</p>
              <div className="session-delete-summary">
                <span>数量：{purgeConfirm.deleteIds.length}</span>
                <span>总大小：{formatSize(purgeConfirm.totalSize)}</span>
              </div>
              <div className="session-delete-list">
                {purgeConfirm.items.map(item => (
                  <div key={item.delete_id} className="session-delete-item">
                    <strong title={item.title}>{item.title}</strong>
                    <span>{formatTime(item.deleted_at)} · {formatSize(item.size_bytes)}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        />
      )}
    </div>
  );
}
