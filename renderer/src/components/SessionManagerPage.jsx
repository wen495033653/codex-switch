import { useEffect, useMemo, useRef, useState } from 'react';
import { useI18n } from '../i18n';
import { getRawErrorMessage } from '../utils/errors';
import {
  PREVIEW_PAGE_SIZE,
  PREVIEW_MESSAGE_WINDOW,
  formatSize,
  displayPath,
  formatTime,
  statusLabel,
  statusActionLabel,
  lower,
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
import SessionScanReport from './session/SessionScanReport';

export default function SessionManagerPage({ toast, toastError, sessionState }) {
  const { language, t } = useI18n();
  const {
    rootPath,
    setRootPath,
    conversations,
    setConversations,
    scanReport,
    setScanReport,
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
    activePath,
    setActivePath,
    preview,
    setPreview,
    contextMenu,
    setContextMenu,
    deleteConfirm,
    setDeleteConfirm,
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
    scanRequestRef,
    hasAutoLoadedRef
  } = sessionState;
  const [previewEarlierLoading, setPreviewEarlierLoading] = useState(false);
  const [previewTrimmedNewer, setPreviewTrimmedNewer] = useState(false);
  const previewItemRef = useRef(null);
  // The preview shown now; a refresh that finishes later checks this, not the value it started with.
  const activePathRef = useRef(activePath);

  const visibleItems = useMemo(() => {
    const query = lower(search.trim());
    return conversations.filter(item => {
      if (statusFilter !== 'all' && item.status !== statusFilter) return false;
      if (!query) return true;
      return lower(item.title).includes(query)
        || lower(item.id).includes(query)
        || lower(item.cwd).includes(query)
        || lower(item.relative_path).includes(query);
    });
  }, [conversations, search, statusFilter]);

  const totalPages = Math.max(1, Math.ceil(visibleItems.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const pageStart = (currentPage - 1) * pageSize;
  const pageItems = visibleItems.slice(pageStart, pageStart + pageSize);
  const selectedPaths = useMemo(() => Array.from(selected), [selected]);

  const selectedItems = useMemo(
    () => conversations.filter(item => selected.has(item.relative_path)),
    [conversations, selected]
  );

  const selectedActivePaths = useMemo(
    () => selectedItems.filter(item => item.status === 'active').map(item => item.relative_path),
    [selectedItems]
  );
  const selectedArchivedPaths = useMemo(
    () => selectedItems.filter(item => item.status === 'archived').map(item => item.relative_path),
    [selectedItems]
  );

  const selectedSize = selectedItems.reduce((sum, item) => sum + (Number(item.size_bytes) || 0), 0);
  const allPageSelected = pageItems.length > 0 && pageItems.every(item => selected.has(item.relative_path));

  const scrollPreviewToBottom = () => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const node = previewRef.current;
        if (node) node.scrollTop = node.scrollHeight;
      });
    });
  };

  const requestPreviewPage = (item, options) => window.api.previewSession({
    root: rootPath,
    relativePath: item.relative_path,
    beforeCursor: options.beforeCursor ?? null,
    snapshotSize: options.snapshotSize ?? null,
    limit: PREVIEW_PAGE_SIZE,
    messageSource: options.messageSource || null,
    requestId: options.requestId
  });

  const loadPreview = async (item) => {
    if (!item || !rootPath) return;
    const requestId = nextPreviewRequestId(previewRequestRef);
    previewItemRef.current = item;
    activePathRef.current = item.relative_path;
    setActivePath(activePathRef.current);
    setPreviewTrimmedNewer(false);
    setPreviewEarlierLoading(false);
    setPreview({
      conversation: item,
      messages: [],
      message_page: null,
      parse_error: item.parse_error || null
    });

    if (item.parse_error && Number(item.size_bytes) === 0) {
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
      setPreview(res);
      scrollPreviewToBottom();
    } catch (err) {
      if (previewRequestRef.current !== requestId || isPreviewCancellation(err)) return;
      const previewErrorMessage = t('读取会话预览失败');
      setPreview(current => ({
        ...(current || {}),
        conversation: item,
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
    const requestId = ++scanRequestRef.current;
    setLoading(true);
    try {
      const res = await window.api.scanSessions(nextRoot || undefined);
      if (scanRequestRef.current !== requestId) return null;
      const nextConversations = Array.isArray(res.conversations) ? res.conversations : [];
      const resolvedRoot = res.root || nextRoot || '';
      const existing = new Set(nextConversations.map(item => item.relative_path));
      setRootPath(resolvedRoot);
      setConversations(nextConversations);
      setScanReport({
        hasResult: true,
        errors: res.errors ?? [],
        warnings: res.warnings ?? [],
        refreshError: null
      });
      setSelected(prev => new Set(Array.from(prev).filter(path => existing.has(path))));
      if (activePathRef.current && !existing.has(activePathRef.current)) {
        previewRequestRef.current += 1;
        previewItemRef.current = null;
        activePathRef.current = '';
        setActivePath('');
        setPreview(null);
        setPreviewLoading(false);
        setPreviewEarlierLoading(false);
        setPreviewTrimmedNewer(false);
      }
      return { root: resolvedRoot, conversations: nextConversations };
    } catch (err) {
      if (scanRequestRef.current !== requestId) return null;
      setScanReport(current => ({ ...current, refreshError: getRawErrorMessage(err) }));
      toastError(err, t('扫描会话失败'), 7000);
      return null;
    } finally {
      if (scanRequestRef.current === requestId) setLoading(false);
    }
  };

  useEffect(() => {
    if (hasAutoLoadedRef.current) return;
    hasAutoLoadedRef.current = true;
    refreshSessions('');
  }, []);

  // The page survives leaving this view, so only an actual filter change goes back to page 1.
  const changeSearch = value => {
    setSearch(value);
    setPage(1);
  };
  const changeStatusFilter = value => {
    if (value === statusFilter) return;
    setStatusFilter(value);
    setPage(1);
  };
  const changePageSize = value => {
    setPageSize(value);
    setPage(1);
  };

  useEffect(() => {
    if (page > totalPages) setPage(totalPages);
  }, [page, totalPages]);

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
    activePathRef.current = '';
    setContextMenu(null);
    setActivePath('');
    setPreview(null);
  }, []);

  const toggleSelection = (item, checked) => {
    setSelected(prev => {
      const next = new Set(prev);
      if (checked) next.add(item.relative_path);
      else next.delete(item.relative_path);
      return next;
    });
  };

  const toggleSelectFiltered = () => {
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

  const clearSelection = () => setSelected(new Set());

  const runAction = async (action, successMessage, errorMessage) => {
    if (actionLoading) return null;
    setActionLoading(true);
    try {
      const res = await action();
      if (res && res.report && res.report.conflict_action_required) return res;
      if (res?.ok === false) toastError(res, errorMessage, 7000);
      else toast(res?.message || successMessage);
      await refreshSessions();
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
    const pathSet = new Set(paths);
    const items = conversations.filter(item => pathSet.has(item.relative_path));
    if (items.length === 0) {
      toast(t('请先选择要删除的会话'));
      return;
    }
    setDeleteConfirm({
      items,
      totalSize: items.reduce((sum, item) => sum + (Number(item.size_bytes) || 0), 0)
    });
  };

  const cancelDeleteSessions = () => {
    if (!actionLoading) setDeleteConfirm(null);
  };

  const confirmDeleteSessions = async () => {
    if (!deleteConfirm || actionLoading) return;
    const threadIds = deleteConfirm.items.map(item => item.id);
    await runAction(
      () => window.api.deleteSessions({ root: rootPath, threadIds }),
      t('删除会话完成'),
      t('删除会话失败')
    );
    setDeleteConfirm(null);
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
        handleImport={handleImport}
        loading={loading}
        refreshSessions={refreshSessions}
        rootPath={rootPath}
        search={search}
        setSearch={changeSearch}
        setStatusFilter={changeStatusFilter}
        statusFilter={statusFilter}
      />

      <SessionScanReport readableCount={conversations.length} report={scanReport} />

      <div className="session-workspace">
        <SessionListPanel
          actionLoading={actionLoading}
          activePath={activePath}
          allPageSelected={allPageSelected}
          clearSelection={clearSelection}
          conversations={conversations}
          currentPage={currentPage}
          handleDeleteSessions={handleDeleteSessions}
          handleExport={handleExport}
          handleRowKeyDown={handleRowKeyDown}
          handleSetStatus={handleSetStatus}
          loadPreview={loadPreview}
          openContextMenu={openContextMenu}
          pageItems={pageItems}
          pageSize={pageSize}
          scanIncomplete={scanReport.errors.length > 0}
          selected={selected}
          selectedActivePaths={selectedActivePaths}
          selectedArchivedPaths={selectedArchivedPaths}
          selectedCount={selectedPaths.length}
          selectedSize={selectedSize}
          setPage={setPage}
          setPageSize={changePageSize}
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
          title={t('确认永久删除会话')}
          width="460px"
          confirmText={t('永久删除')}
          loadingText={t('删除中...')}
          confirmVariant="danger"
          isLoading={actionLoading}
          onCancel={cancelDeleteSessions}
          onConfirm={confirmDeleteSessions}
          content={(
            <div className="session-delete-confirm">
              <p>{t('将永久删除所选 {count} 个会话及其子代理会话，删除后不可恢复。', { count: deleteConfirm.items.length })}</p>
              <div className="session-delete-summary">
                <span>{t('数量：{count}', { count: deleteConfirm.items.length })}</span>
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
    </div>
  );
}
