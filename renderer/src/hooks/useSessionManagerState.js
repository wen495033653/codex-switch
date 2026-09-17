import { useRef, useState } from 'react';

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
