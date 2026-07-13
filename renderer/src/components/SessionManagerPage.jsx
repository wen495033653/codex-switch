import { useEffect, useMemo, useRef, useState } from 'react';
import { useI18n } from '../i18n';
import ConfirmDialog from './ConfirmDialog';

const STATUS_FILTERS = [
  { key: 'all', label: '全部' },
  { key: 'active', label: '未归档' },
  { key: 'archived', label: '已归档' },
  { key: 'deleted', label: '已删除' }
];

const PAGE_SIZE_OPTIONS = [50, 100, 200];
const PREVIEW_PAGE_SIZE = 80;
const PREVIEW_MESSAGE_WINDOW = 240;
const DELETE_UNDO_WINDOW_MS = 10_000;

function formatSize(bytes) {
  const value = Number(bytes) || 0;
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${value} B`;
}

function displayPath(value) {
  const path = String(value || '').trim();
  if (/^\\\\\?\\UNC\\/i.test(path)) return `\\\\${path.slice(8)}`;
  if (/^\\\\\?\\/.test(path)) return path.slice(4);
  return path;
}

function formatTime(value, language, t) {
  if (!value) return t('未知');
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(language);
}

function statusLabel(status, t) {
  if (status === 'archived') return t('已归档');
  if (status === 'deleted') return t('已删除');
  return t('未归档');
}

function statusActionLabel(status, t) {
  return status === 'archived' ? t('归档') : t('取消归档');
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

function responseDeleteIds(response) {
  const ids = Array.isArray(response?.report?.delete_ids)
    ? response.report.delete_ids
    : response?.delete_ids;
  return Array.from(new Set((Array.isArray(ids) ? ids : []).filter(Boolean)));
}

function responseRestoredDeleteIds(response) {
  const ids = Array.isArray(response?.report?.restored_delete_ids)
    ? response.report.restored_delete_ids
    : response?.restored_delete_ids;
  return Array.from(new Set((Array.isArray(ids) ? ids : []).filter(Boolean)));
}

function responsePurgedDeleteIds(response) {
  const ids = response?.report?.purged_delete_ids;
  return Array.from(new Set((Array.isArray(ids) ? ids : []).filter(Boolean)));
}

function previewMessageKey(message, fallbackIndex = 0) {
  if (message && message.offset !== undefined && message.offset !== null) {
    return `${message.role || 'message'}:${message.offset}`;
  }
  return `${message?.role || 'message'}:${message?.timestamp || 'unknown'}:${fallbackIndex}`;
}

function isPreviewCancellation(error) {
  return String(error?.message || error || '').includes('会话预览请求已取消');
}

function isPreviewStale(error) {
  return String(error?.message || error || '').includes('会话文件已变化');
}

function nextPreviewRequestId(requestRef) {
  const candidate = (Date.now() * 1000) + ((requestRef.current + 1) % 1000);
  requestRef.current = Math.max(requestRef.current + 1, candidate);
  return requestRef.current;
}

export function useSessionManagerState() {
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
  const [deleteConfirm, setDeleteConfirm] = useState(null);
  const [purgeConfirm, setPurgeConfirm] = useState(null);
  const [deleteUndo, setDeleteUndo] = useState(null);
  const [conflictConfirm, setConflictConfirm] = useState(null);
  const [loading, setLoading] = useState(false);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [actionLoading, setActionLoading] = useState(false);
  const previewRef = useRef(null);
  const previewRequestRef = useRef(0);
  const hasAutoLoadedRef = useRef(false);

  return {
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
  };
}

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
      <div className="session-filterbar">
        <div className="search-wrapper session-search">
          <svg className="search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-3.5-3.5" />
          </svg>
          <input
            className="search-input"
            placeholder={t('搜索标题、ID、工作目录或路径...')}
            aria-label={t('搜索标题、ID、工作目录或路径...')}
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
                  aria-pressed={statusFilter === item.key}
                  onClick={() => setStatusFilter(item.key)}
                >
                  <span className="session-status-label">{t(item.label)}</span>
                  <span className="session-status-count">{count}</span>
                </button>
            );
          })}
        </div>
        <div className="session-page-actions">
          <button type="button" className="btn btn-secondary" onClick={() => refreshSessions(rootPath)} disabled={loading || actionLoading}>
            {loading ? t('刷新中...') : t('刷新')}
          </button>
          {!isDeletedView && (
            <button type="button" className="btn btn-primary" onClick={handleImport} disabled={!rootPath || actionLoading}>
              {t('导入会话')}
            </button>
          )}
        </div>
      </div>

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
        <div className="session-list-panel">
          <div className="session-list-header">
            <label className="session-checkbox">
              <input
                type="checkbox"
                aria-label={t('选择本页会话')}
                checked={allPageSelected}
                onChange={toggleSelectFiltered}
              />
            </label>
            <span>{t('标题')}</span>
            <span>{t('状态')}</span>
            <span>{isDeletedView ? t('删除时间') : t('更新时间')}</span>
            <span>{t('大小')}</span>
          </div>
          <div className="session-list-body">
            <div className={`session-list ${selectedCount > 0 ? 'has-batch-actions' : ''}`}>
              {pageItems.map(item => {
                const rowKey = isDeletedView ? item.delete_id : item.relative_path;
                const activeKey = isDeletedView ? deletedActiveKey(item) : item.relative_path;
                const isSelected = isDeletedView
                  ? selectedDeleted.has(item.delete_id)
                  : selected.has(item.relative_path);
                return (
                  <div
                    key={rowKey}
                    role="button"
                    tabIndex={0}
                    aria-label={item.title}
                    className={`session-row ${activePath === activeKey ? 'active' : ''}`}
                    onClick={() => loadPreview(item)}
                    onKeyDown={event => handleRowKeyDown(event, item)}
                    onContextMenu={event => openContextMenu(event, item)}
                  >
                    <span className="session-checkbox" onClick={event => event.stopPropagation()}>
                      <input
                        type="checkbox"
                        aria-label={t('选择会话：{title}', { title: item.title || t('未命名') })}
                        checked={isSelected}
                        onChange={event => toggleSelection(item, event.target.checked)}
                      />
                    </span>
                    <span className="session-title-cell">
                      <strong title={item.title}>{item.title}</strong>
                    </span>
                    <span className={`session-status-pill ${item.status}`}>{statusLabel(item.status, t)}</span>
                    <span className="session-muted">
                      {formatTime(isDeletedView ? item.deleted_at : item.updated_at, language, t)}
                    </span>
                    <span className="session-muted">{formatSize(item.size_bytes)}</span>
                  </div>
                );
              })}
              {visibleItems.length === 0 && (
                <div className="empty-state session-empty">
                  {isDeletedView ? t('暂无已删除会话') : t('暂无会话数据')}
                </div>
              )}
            </div>
            {selectedCount > 0 && (
              <div className="session-contextual-toolbar" role="toolbar" aria-label={t('会话批量操作')}>
                <span className="session-batch-count">{t('已选 {count}', { count: selectedCount })}</span>
                {isDeletedView ? (
                  <>
                    <button type="button" className="btn btn-secondary" onClick={() => handleRestoreDeleted()} disabled={actionLoading}>
                      {t('恢复')}
                    </button>
                    <button type="button" className="btn btn-danger" onClick={() => handlePurgeDeleted()} disabled={actionLoading}>
                      {t('彻底删除')}
                    </button>
                  </>
                ) : (
                  <>
                    <button type="button" className="btn btn-secondary" onClick={() => handleExport()} disabled={actionLoading}>
                      {t('导出')}
                    </button>
                    {selectedActivePaths.length > 0 && (
                      <button type="button" className="btn btn-secondary" onClick={() => handleSetStatus(selectedActivePaths, 'archived')} disabled={actionLoading}>
                        {t('归档')}
                      </button>
                    )}
                    {selectedArchivedPaths.length > 0 && (
                      <button type="button" className="btn btn-secondary" onClick={() => handleSetStatus(selectedArchivedPaths, 'active')} disabled={actionLoading}>
                        {t('取消归档')}
                      </button>
                    )}
                    <button type="button" className="btn btn-danger" onClick={() => handleDeleteSessions()} disabled={actionLoading}>
                      {t('删除')}
                    </button>
                  </>
                )}
                <button type="button" className="btn btn-secondary" onClick={clearSelection} disabled={actionLoading}>
                  {t('取消选择')}
                </button>
              </div>
            )}
          </div>
          <div className="session-footer">
            <span>{t('总计 {count} 个', { count: isDeletedView ? deletedSessions.length : conversations.length })}</span>
            <span>{t('筛选 {count} 个', { count: visibleItems.length })}</span>
            <span>{t('本页 {count} 个', { count: pageItems.length })}</span>
            <span>{t('已选 {count} 个', { count: selectedCount })}</span>
            <span>{formatSize(selectedSize)}</span>
            <div className="session-pagination">
              <button type="button" className="btn btn-secondary" onClick={() => setPage(1)} disabled={currentPage <= 1}>
                {t('首页')}
              </button>
              <button type="button" className="btn btn-secondary" onClick={() => setPage(value => Math.max(1, value - 1))} disabled={currentPage <= 1}>
                {t('上页')}
              </button>
              <span>{currentPage}/{totalPages}</span>
              <button type="button" className="btn btn-secondary" onClick={() => setPage(value => Math.min(totalPages, value + 1))} disabled={currentPage >= totalPages}>
                {t('下页')}
              </button>
              <select value={pageSize} onChange={event => setPageSize(Number(event.target.value))}>
                {PAGE_SIZE_OPTIONS.map(value => (
                  <option key={value} value={value}>{t('每页 {count}', { count: value })}</option>
                ))}
              </select>
            </div>
          </div>
        </div>

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
      </div>

      {contextMenu && (
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
      )}

      {conflictConfirm && (
        <div className="modal-overlay">
          <div className="modal-content modal-content-lg session-conflict-dialog" role="dialog" aria-modal="true" aria-labelledby="session-conflict-title" aria-describedby="session-conflict-message">
            <h3 id="session-conflict-title">{conflictConfirm.title}</h3>
            <p id="session-conflict-message">{conflictConfirm.message}</p>
            <div className="session-conflict-list">
              {conflictConfirm.conflicts.slice(0, 8).map((item, index) => (
                <div key={`${item.target || item.relative_path || item.delete_id || index}`} className="session-conflict-item">
                  <strong title={item.title || item.target || ''}>{item.title || item.target || t('冲突会话')}</strong>
                  <span title={item.target || item.relative_path || item.delete_id || ''}>
                    {item.target || item.relative_path || item.delete_id}
                  </span>
                </div>
              ))}
              {conflictConfirm.conflicts.length > 8 && (
                <div className="session-conflict-item">
                  <strong>{t('还有 {count} 个冲突', { count: conflictConfirm.conflicts.length - 8 })}</strong>
                </div>
              )}
            </div>
            <div className="session-conflict-actions">
              <button type="button" className="btn btn-secondary" onClick={() => resolveConflictDialog('skip')} disabled={actionLoading}>
                {t('跳过')}
              </button>
              <button type="button" className="btn btn-secondary" onClick={() => resolveConflictDialog('modify_id')} disabled={actionLoading}>
                {t('修改 ID')}
              </button>
              <button type="button" className="btn btn-danger" onClick={() => resolveConflictDialog('overwrite')} disabled={actionLoading}>
                {t('覆盖')}
              </button>
            </div>
          </div>
        </div>
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
