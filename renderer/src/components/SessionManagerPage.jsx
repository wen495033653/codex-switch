import { useEffect, useMemo, useRef, useState } from 'react';
import { useI18n } from '../i18n';
import {
  PREVIEW_PAGE_SIZE,
  PREVIEW_MESSAGE_WINDOW,
  DELETE_UNDO_WINDOW_MS,
  formatSize,
  displayPath,
  formatTime,
  statusLabel,
  statusActionLabel,
  lower,
  deletedActiveKey,
  deletedPreviewConversation,
  responseDeleteIds,
  responseRestoredDeleteIds,
  responsePurgedDeleteIds,
  previewMessageKey,
  isPreviewCancellation,
  isPreviewStale,
  nextPreviewRequestId,
} from '../utils/sessionManager';
import ConfirmDialog from './ConfirmDialog';
import SessionConflictDialog from './session/SessionConflictDialog';
import SessionContextMenu from './session/SessionContextMenu';
import SessionFilterBar from './session/SessionFilterBar';
import SessionListPanel from './session/SessionListPanel';
import SessionPreviewPanel from './session/SessionPreviewPanel';

export default function SessionManagerPage({ toast, toastError, sessionState }) {
  const { language, t, translateRuntimeText } = useI18n();
  const {
    rootPath,
    setRootPath,
    conversations,
    setConversations,
    deletedSessions,
    setDeletedSessions,
    search,
    setSearch,
    statusFilter,
    setStatusFilter,
    page,
    setPage,
    pageSize,
    setPageSize,
    selected,
    setSelected,
    selectedDeleted,
    setSelectedDeleted,
    activePath,
    setActivePath,
    preview,
    setPreview,
    contextMenu,
    setContextMenu,
    deleteConfirm,
    setDeleteConfirm,
    purgeConfirm,
    setPurgeConfirm,
    deleteUndo,
    setDeleteUndo,
    conflictConfirm,
    setConflictConfirm,
    loading,
    setLoading,
    previewLoading,
    setPreviewLoading,
    actionLoading,
    setActionLoading,
    previewRef,
    previewRequestRef,
    hasAutoLoadedRef
  } = sessionState;
  const [previewEarlierLoading, setPreviewEarlierLoading] = useState(false);
  const [previewTrimmedNewer, setPreviewTrimmedNewer] = useState(false);
  const previewItemRef = useRef(null);
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

  const selectedActivePaths = useMemo(
    () => selectedItems.filter(item => item.status === 'active').map(item => item.relative_path),
    [selectedItems]
  );
  const selectedArchivedPaths = useMemo(
    () => selectedItems.filter(item => item.status === 'archived').map(item => item.relative_path),
    [selectedItems]
  );

  const selectedDeletedItems = useMemo(() => {
    const selectedSet = new Set(selectedDeletedIds);
    return deletedSessions.filter(item => selectedSet.has(item.delete_id));
  }, [deletedSessions, selectedDeletedIds]);

  const selectedSize = (isDeletedView ? selectedDeletedItems : selectedItems)
    .reduce((sum, item) => sum + (Number(item.size_bytes) || 0), 0);
  const selectedCount = isDeletedView ? selectedDeletedIds.length : selectedPaths.length;
  const allPageSelected = pageItems.length > 0
    && pageItems.every(item => (
      isDeletedView ? selectedDeleted.has(item.delete_id) : selected.has(item.relative_path)
    ));

  const scrollPreviewToBottom = () => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const node = previewRef.current;
        if (node) node.scrollTop = node.scrollHeight;
      });
    });
  };

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
      toastError(err, t('读取已删除会话失败'), 6000);
      return deletedSessions;
    }
  };

  const requestPreviewPage = (item, options) => {
    const payload = {
      beforeCursor: options.beforeCursor ?? null,
      snapshotSize: options.snapshotSize ?? null,
      limit: PREVIEW_PAGE_SIZE,
      messageSource: options.messageSource || null,
      requestId: options.requestId
    };
    if (item.status === 'deleted') {
      return window.api.previewDeletedSession({
        ...payload,
        deleteId: item.delete_id
      });
    }
    return window.api.previewSession({
      ...payload,
      root: rootPath,
      relativePath: item.relative_path
    });
  };

  const loadPreview = async (item) => {
    if (!item || (item.status !== 'deleted' && !rootPath)) return;
    const requestId = nextPreviewRequestId(previewRequestRef);
    previewItemRef.current = item;
    setActivePath(item.status === 'deleted' ? deletedActiveKey(item) : item.relative_path);
    setPreviewTrimmedNewer(false);
    setPreviewEarlierLoading(false);
    setPreview({
      conversation: item.status === 'deleted' ? deletedPreviewConversation(item) : item,
      messages: [],
      message_page: null,
      parse_error: item.parse_error || null
    });

    if (item.status !== 'deleted' && item.parse_error && Number(item.size_bytes) === 0) {
      setPreviewLoading(false);
      return;
    }

    setPreviewLoading(true);
    try {
      const res = await requestPreviewPage(item, {
        beforeCursor: null,
        snapshotSize: null,
        messageSource: null,
        requestId
      });
      if (previewRequestRef.current !== requestId) return;
      setPreview(item.status === 'deleted'
        ? {
            ...res,
            conversation: {
              ...deletedPreviewConversation(item),
              ...(res.conversation || {}),
              status: 'deleted'
            }
          }
        : res);
      scrollPreviewToBottom();
    } catch (err) {
      if (previewRequestRef.current !== requestId || isPreviewCancellation(err)) return;
      const previewErrorMessage = item.status === 'deleted'
        ? t('读取已删除会话预览失败')
        : t('读取会话预览失败');
      setPreview(current => ({
        ...(current || {}),
        conversation: item.status === 'deleted' ? deletedPreviewConversation(item) : item,
        messages: [],
        parse_error: err && err.message
          ? err.message
          : String(err || previewErrorMessage)
      }));
      toastError(
        err,
        previewErrorMessage,
        6000
      );
    } finally {
      if (previewRequestRef.current === requestId) setPreviewLoading(false);
    }
  };

  const loadEarlierMessages = async () => {
    const item = previewItemRef.current;
    const pageState = preview?.message_page;
    if (!item || !pageState?.has_more || pageState.next_before === null || previewEarlierLoading) return;

    const requestId = nextPreviewRequestId(previewRequestRef);
    setPreviewEarlierLoading(true);
    const listNode = previewRef.current;
    const anchorOffset = preview?.messages?.[0]?.offset;
    const anchorNode = anchorOffset !== undefined && listNode
      ? listNode.querySelector(`[data-message-offset="${anchorOffset}"]`)
      : null;
    const anchorTop = anchorNode ? anchorNode.getBoundingClientRect().top : null;

    try {
      const res = await requestPreviewPage(item, {
        beforeCursor: pageState.next_before,
        snapshotSize: pageState.file_size,
        messageSource: pageState.source,
        requestId
      });
      if (previewRequestRef.current !== requestId) return;
      const incomingCount = Array.isArray(res.messages) ? res.messages.length : 0;
      setPreviewTrimmedNewer(value => (
        value || ((preview?.messages?.length || 0) + incomingCount > PREVIEW_MESSAGE_WINDOW)
      ));
      setPreview(current => {
        if (!current) return current;
        const existingKeys = new Set((current.messages || []).map(previewMessageKey));
        const older = (Array.isArray(res.messages) ? res.messages : [])
          .filter((message, index) => !existingKeys.has(previewMessageKey(message, index)));
        let messages = [...older, ...(current.messages || [])];
        if (messages.length > PREVIEW_MESSAGE_WINDOW) {
          messages = messages.slice(0, PREVIEW_MESSAGE_WINDOW);
        }
        return {
          ...current,
          messages,
          message_page: res.message_page,
          parse_error: res.parse_error || current.parse_error
        };
      });
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          if (anchorTop === null || anchorOffset === undefined || !previewRef.current) return;
          const nextAnchor = previewRef.current.querySelector(`[data-message-offset="${anchorOffset}"]`);
          if (!nextAnchor) return;
          previewRef.current.scrollTop += nextAnchor.getBoundingClientRect().top - anchorTop;
        });
      });
    } catch (err) {
      if (previewRequestRef.current !== requestId || isPreviewCancellation(err)) return;
      if (isPreviewStale(err)) {
        loadPreview(item);
        return;
      }
      if (!isPreviewCancellation(err)) toastError(err, t('加载更早消息失败'), 5000);
    } finally {
      if (previewRequestRef.current === requestId) setPreviewEarlierLoading(false);
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
      toastError(err, t('扫描会话失败'), 7000);
    }
    nextDeleted = await loadDeletedSessions();
    if (activePath) {
      const activeExists = nextConversations.some(item => item.relative_path === activePath)
        || nextDeleted.some(item => deletedActiveKey(item) === activePath);
      if (!activeExists) {
        previewRequestRef.current += 1;
        previewItemRef.current = null;
        setActivePath('');
        setPreview(null);
        setPreviewTrimmedNewer(false);
      }
    }
    setLoading(false);
  };

  useEffect(() => {
    if (hasAutoLoadedRef.current) return;
    hasAutoLoadedRef.current = true;
    refreshSessions('');
  }, []);

  useEffect(() => {
    setPage(1);
  }, [search, statusFilter, pageSize]);

  useEffect(() => {
    if (page > totalPages) setPage(totalPages);
  }, [page, totalPages]);

  useEffect(() => {
    if (!deleteUndo) return undefined;
    const remaining = deleteUndo.expiresAt - Date.now();
    if (remaining <= 0) {
      setDeleteUndo(null);
      return undefined;
    }
    const timer = window.setTimeout(() => setDeleteUndo(null), remaining);
    return () => window.clearTimeout(timer);
  }, [deleteUndo, setDeleteUndo]);

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

  useEffect(() => () => {
    previewRequestRef.current += 1;
    previewItemRef.current = null;
    setContextMenu(null);
    setActivePath('');
    setPreview(null);
  }, []);

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
      toast(t('请先选择要导出的会话'));
      return;
    }
    runAction(
      () => window.api.exportSessions({ root: rootPath, relativePaths: paths }),
      t('导出会话完成'),
      t('导出会话失败')
    );
  };

  const handleImport = () => {
    runAction(() => window.api.importSessions(rootPath), t('导入会话完成'), t('导入会话失败'));
  };

  const handleSetStatus = (paths, targetStatus, conflictStrategy = 'ask') => {
    const nextPaths = Array.from(new Set(paths));
    if (nextPaths.length === 0) {
      toast(targetStatus === 'archived' ? t('请先选择要归档的会话') : t('请先选择要取消归档的会话'));
      return;
    }
    runAction(
      () => window.api.setSessionStatus({
        root: rootPath,
        relativePaths: nextPaths,
        status: targetStatus,
        conflictStrategy
      }),
      t('{action}完成', { action: statusActionLabel(targetStatus, t) }),
      t('{action}失败', { action: statusActionLabel(targetStatus, t) })
    ).then(res => {
      if (res && res.report && res.report.conflict_action_required) {
        openConflictDialog({
          title: t('{action}存在冲突', { action: statusActionLabel(targetStatus, t) }),
          message: t('目标位置已有同名会话文件，请选择这批冲突的处理方式。'),
          conflicts: res.report.conflicts,
          onResolve: strategy => handleSetStatus(nextPaths, targetStatus, strategy)
        });
      }
    });
  };

  const handleDeleteSessions = (paths = selectedPaths) => {
    const nextPaths = Array.from(new Set(paths));
    if (nextPaths.length === 0) {
      toast(t('请先选择要删除的会话'));
      return;
    }
    const pathSet = new Set(nextPaths);
    const items = conversations.filter(item => pathSet.has(item.relative_path));
    setDeleteConfirm({
      paths: nextPaths,
      items,
      totalSize: items.reduce((sum, item) => sum + (Number(item.size_bytes) || 0), 0)
    });
  };

  const cancelDeleteSessions = () => {
    if (!actionLoading) setDeleteConfirm(null);
  };

  const confirmDeleteSessions = () => {
    if (!deleteConfirm || actionLoading) return;
    const paths = deleteConfirm.paths;
    const deleteRoot = rootPath;
    runAction(
      () => window.api.deleteSessions({ root: deleteRoot, relativePaths: paths }),
      t('删除会话完成'),
      t('删除会话失败')
    ).then(res => {
      const deleteIds = responseDeleteIds(res);
      if (deleteIds.length > 0) {
        const now = Date.now();
        setDeleteUndo({
          root: deleteRoot,
          deleteIds,
          expiresAt: now + DELETE_UNDO_WINDOW_MS
        });
      }
      setDeleteConfirm(null);
    });
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

  const handleRestoreDeleted = (
    deleteIds = selectedDeletedIds,
    conflictStrategy = 'ask',
    options = {}
  ) => {
    const ids = Array.from(new Set(deleteIds));
    if (ids.length === 0) {
      toast(t('请先选择要恢复的会话'));
      return;
    }
    const idSet = new Set(ids);
    const items = deletedSessions.filter(item => idSet.has(item.delete_id));
    const targetRoot = options.root || rootPath || items[0]?.root_path || '';
    if (!targetRoot) {
      toast(t('请先选择 Codex 数据目录'));
      return;
    }
    runAction(
      () => window.api.restoreDeletedSessions({
        root: targetRoot,
        deleteIds: ids,
        conflictStrategy
      }),
      t('恢复会话完成'),
      t('恢复会话失败'),
      targetRoot
    ).then(res => {
      if (res && res.report && res.report.conflict_action_required) {
        openConflictDialog({
          title: t('恢复会话存在冲突'),
          message: t('恢复目标位置已有会话文件，请选择这批冲突的处理方式。'),
          conflicts: res.report.conflicts,
          onResolve: strategy => handleRestoreDeleted(ids, strategy, options)
        });
        return;
      }
      if (!res) return;
      const restoredIds = responseRestoredDeleteIds(res);
      if (restoredIds.length > 0) {
        const restoredSet = new Set(restoredIds);
        setSelectedDeleted(prev => new Set(
          Array.from(prev).filter(id => !restoredSet.has(id))
        ));
        setDeleteUndo(current => {
          if (!current) return current;
          const remainingIds = current.deleteIds.filter(id => !restoredSet.has(id));
          return remainingIds.length > 0
            ? { ...current, deleteIds: remainingIds }
            : null;
        });
      }
    });
  };

  const handleUndoDelete = () => {
    if (!deleteUndo) return;
    if (deleteUndo.expiresAt <= Date.now()) {
      setDeleteUndo(null);
      return;
    }
    handleRestoreDeleted(deleteUndo.deleteIds, 'ask', {
      root: deleteUndo.root
    });
  };

  const handlePurgeDeleted = (deleteIds = selectedDeletedIds) => {
    const ids = Array.from(new Set(deleteIds));
    if (ids.length === 0) {
      toast(t('请先选择要彻底删除的会话'));
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
      t('彻底删除完成'),
      t('彻底删除失败')
    ).then(res => {
      if (res) {
        const purgedIds = responsePurgedDeleteIds(res);
        const purgedSet = new Set(purgedIds);
        setSelectedDeleted(prev => new Set(Array.from(prev).filter(id => !purgedSet.has(id))));
        setDeleteUndo(current => {
          if (!current) return current;
          const remainingIds = current.deleteIds.filter(id => !purgedSet.has(id));
          return remainingIds.length > 0
            ? { ...current, deleteIds: remainingIds }
            : null;
        });
      }
      setPurgeConfirm(null);
    });
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

  const handleRowKeyDown = (event, item) => {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    loadPreview(item);
  };

  const activeConversation = preview && preview.conversation ? preview.conversation : null;
  const messages = preview && Array.isArray(preview.messages) ? preview.messages : [];
  const activeCwd = displayPath(activeConversation?.cwd);
  const activeSourcePath = displayPath(activeConversation?.source_path);

  return (
    <div className="session-manager-page">
      <SessionFilterBar
        actionLoading={actionLoading}
        conversations={conversations}
        deletedSessions={deletedSessions}
        handleImport={handleImport}
        isDeletedView={isDeletedView}
        loading={loading}
        refreshSessions={refreshSessions}
        rootPath={rootPath}
        search={search}
        setSearch={setSearch}
        setStatusFilter={setStatusFilter}
        statusFilter={statusFilter}
      />

      {deleteUndo && (
        <div className="session-delete-undo" role="status" aria-live="polite">
          <span>{t('已删除 {count} 个会话', { count: deleteUndo.deleteIds.length })}</span>
          <button
            type="button"
            className="session-delete-undo-action"
            onClick={handleUndoDelete}
            disabled={actionLoading}
          >
            {t('撤销')}
          </button>
        </div>
      )}

      <div className="session-workspace">
        <SessionListPanel
          actionLoading={actionLoading}
          activePath={activePath}
          allPageSelected={allPageSelected}
          clearSelection={clearSelection}
          conversations={conversations}
          currentPage={currentPage}
          deletedSessions={deletedSessions}
          handleDeleteSessions={handleDeleteSessions}
          handleExport={handleExport}
          handlePurgeDeleted={handlePurgeDeleted}
          handleRestoreDeleted={handleRestoreDeleted}
          handleRowKeyDown={handleRowKeyDown}
          handleSetStatus={handleSetStatus}
          isDeletedView={isDeletedView}
          loadPreview={loadPreview}
          openContextMenu={openContextMenu}
          pageItems={pageItems}
          pageSize={pageSize}
          selected={selected}
          selectedActivePaths={selectedActivePaths}
          selectedArchivedPaths={selectedArchivedPaths}
          selectedCount={selectedCount}
          selectedDeleted={selectedDeleted}
          selectedSize={selectedSize}
          setPage={setPage}
          setPageSize={setPageSize}
          toggleSelectFiltered={toggleSelectFiltered}
          toggleSelection={toggleSelection}
          totalPages={totalPages}
          visibleItems={visibleItems}
        />

        <SessionPreviewPanel
          activeConversation={activeConversation}
          activeCwd={activeCwd}
          activeSourcePath={activeSourcePath}
          loadEarlierMessages={loadEarlierMessages}
          loadPreview={loadPreview}
          messages={messages}
          preview={preview}
          previewEarlierLoading={previewEarlierLoading}
          previewItemRef={previewItemRef}
          previewLoading={previewLoading}
          previewRef={previewRef}
          previewTrimmedNewer={previewTrimmedNewer}
        />
      </div>

      {contextMenu && (
        <SessionContextMenu
          contextMenu={contextMenu}
          handleDeleteSessions={handleDeleteSessions}
          handleExport={handleExport}
          handlePurgeDeleted={handlePurgeDeleted}
          handleRestoreDeleted={handleRestoreDeleted}
          handleSetStatus={handleSetStatus}
          setContextMenu={setContextMenu}
        />
      )}

      {conflictConfirm && (
        <SessionConflictDialog
          actionLoading={actionLoading}
          conflictConfirm={conflictConfirm}
          resolveConflictDialog={resolveConflictDialog}
        />
      )}

      {deleteConfirm && (
        <ConfirmDialog
          title={t('确认删除会话')}
          width="460px"
          confirmText={t('删除')}
          loadingText={t('删除中...')}
          confirmVariant="danger"
          isLoading={actionLoading}
          onCancel={cancelDeleteSessions}
          onConfirm={confirmDeleteSessions}
          content={(
            <div className="session-delete-confirm">
              <p>{t('将 {count} 个会话移入已删除，可在已删除列表中恢复。', { count: deleteConfirm.paths.length })}</p>
              <div className="session-delete-summary">
                <span>{t('数量：{count}', { count: deleteConfirm.paths.length })}</span>
                <span>{t('总大小：{size}', { size: formatSize(deleteConfirm.totalSize) })}</span>
              </div>
              <div className="session-delete-list">
                {deleteConfirm.items.map(item => (
                  <div key={item.relative_path} className="session-delete-item">
                    <strong title={item.title}>{item.title}</strong>
                    <span>
                      {statusLabel(item.status, t)} · {formatTime(item.updated_at, language, t)} · {formatSize(item.size_bytes)}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
        />
      )}

      {purgeConfirm && (
        <ConfirmDialog
          title={t('确认彻底删除')}
          width="460px"
          confirmText={t('彻底删除')}
          loadingText={t('删除中...')}
          confirmVariant="danger"
          isLoading={actionLoading}
          onCancel={cancelPurgeDeleted}
          onConfirm={confirmPurgeDeleted}
          content={(
            <div className="session-delete-confirm">
              <p>{t('将从 Codex Switch 数据目录中彻底删除 {count} 个会话备份。', { count: purgeConfirm.deleteIds.length })}</p>
              <p>{t('彻底删除后无法恢复。')}</p>
              <div className="session-delete-summary">
                <span>{t('数量：{count}', { count: purgeConfirm.deleteIds.length })}</span>
                <span>{t('总大小：{size}', { size: formatSize(purgeConfirm.totalSize) })}</span>
              </div>
              <div className="session-delete-list">
                {purgeConfirm.items.map(item => (
                  <div key={item.delete_id} className="session-delete-item">
                    <strong title={item.title}>{item.title}</strong>
                    <span>{formatTime(item.deleted_at, language, t)} · {formatSize(item.size_bytes)}</span>
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
