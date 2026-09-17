export const STATUS_FILTERS = [
  { key: 'all', label: '全部' },
  { key: 'active', label: '未归档' },
  { key: 'archived', label: '已归档' },
  { key: 'deleted', label: '已删除' }
];

export const PAGE_SIZE_OPTIONS = [50, 100, 200];
export const PREVIEW_PAGE_SIZE = 80;
export const PREVIEW_MESSAGE_WINDOW = 240;
export const DELETE_UNDO_WINDOW_MS = 10_000;

export function formatSize(bytes) {
  const value = Number(bytes) || 0;
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${value} B`;
}

export function displayPath(value) {
  const path = String(value || '').trim();
  if (/^\\\\\?\\UNC\\/i.test(path)) return `\\\\${path.slice(8)}`;
  if (/^\\\\\?\\/.test(path)) return path.slice(4);
  return path;
}

export function formatTime(value, language, t) {
  if (!value) return t('未知');
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(language);
}

export function statusLabel(status, t) {
  if (status === 'archived') return t('已归档');
  if (status === 'deleted') return t('已删除');
  return t('未归档');
}

export function statusActionLabel(status, t) {
  return status === 'archived' ? t('归档') : t('取消归档');
}

export function lower(value) {
  return String(value || '').toLowerCase();
}

export function deletedActiveKey(item) {
  return `deleted:${item.delete_id}`;
}

export function deletedPreviewConversation(item) {
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

export function responseDeleteIds(response) {
  const ids = Array.isArray(response?.report?.delete_ids)
    ? response.report.delete_ids
    : response?.delete_ids;
  return Array.from(new Set((Array.isArray(ids) ? ids : []).filter(Boolean)));
}

export function responseRestoredDeleteIds(response) {
  const ids = Array.isArray(response?.report?.restored_delete_ids)
    ? response.report.restored_delete_ids
    : response?.restored_delete_ids;
  return Array.from(new Set((Array.isArray(ids) ? ids : []).filter(Boolean)));
}

export function responsePurgedDeleteIds(response) {
  const ids = response?.report?.purged_delete_ids;
  return Array.from(new Set((Array.isArray(ids) ? ids : []).filter(Boolean)));
}

export function previewMessageKey(message, fallbackIndex = 0) {
  if (message && message.offset !== undefined && message.offset !== null) {
    return `${message.role || 'message'}:${message.offset}`;
  }
  return `${message?.role || 'message'}:${message?.timestamp || 'unknown'}:${fallbackIndex}`;
}

export function isPreviewCancellation(error) {
  return String(error?.message || error || '').includes('会话预览请求已取消');
}

export function isPreviewStale(error) {
  return String(error?.message || error || '').includes('会话文件已变化');
}

export function nextPreviewRequestId(requestRef) {
  const candidate = (Date.now() * 1000) + ((requestRef.current + 1) % 1000);
  requestRef.current = Math.max(requestRef.current + 1, candidate);
  return requestRef.current;
}
