export const STATUS_FILTERS = [
  { key: 'all', label: '全部' },
  { key: 'active', label: '未归档' },
  { key: 'archived', label: '已归档' }
];

export const PAGE_SIZE_OPTIONS = [50, 100, 200];
export const PREVIEW_PAGE_SIZE = 80;
export const PREVIEW_MESSAGE_WINDOW = 240;

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

// The list column only fits month, day and minutes (or the date for another year); the full
// time from formatTime goes in the cell's title.
export function formatCompactTime(value, language, t) {
  if (!value) return t('未知');
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const options = date.getFullYear() === new Date().getFullYear()
    ? { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false }
    : { year: 'numeric', month: '2-digit', day: '2-digit' };
  return new Intl.DateTimeFormat(language, options).format(date);
}

export function statusLabel(status, t) {
  if (status === 'archived') return t('已归档');
  return t('未归档');
}

export function statusActionLabel(status, t) {
  return status === 'archived' ? t('归档') : t('取消归档');
}

export function lower(value) {
  return String(value || '').toLowerCase();
}

export function filterSessions(conversations, search, statusFilter) {
  const query = lower(search.trim());
  return conversations.filter(item => {
    if (statusFilter !== 'all' && item.status !== statusFilter) return false;
    return !query
      || lower(item.title).includes(query)
      || lower(item.id).includes(query)
      || lower(item.cwd).includes(query)
      || lower(item.relative_path).includes(query);
  });
}

// Bill links identify the thread, never the title, path, or fuzzy search matches.
export function locateSession(conversations, threadId, search, statusFilter, pageSize) {
  const item = conversations.find(conversation => conversation.id === threadId);
  if (!item) return null;
  const index = filterSessions(conversations, search, statusFilter).indexOf(item);
  return index === -1
    ? { item, page: Math.floor(conversations.indexOf(item) / pageSize) + 1, clearFilters: true }
    : { item, page: Math.floor(index / pageSize) + 1, clearFilters: false };
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
