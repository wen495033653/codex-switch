import { useI18n } from '../i18n';

// Footer of a card grid: the visible range and the page buttons. `hideSinglePage` hides the
// buttons while everything fits on one page; the API page opted into that in d1488e3, the
// accounts page never did, so each page keeps its own behavior.
export default function Pagination({
  hideSinglePage = false,
  onPageChange,
  page,
  pageSize,
  startIdx,
  total,
  totalPages
}) {
  const { t } = useI18n();
  const showPageButtons = totalPages > (hideSinglePage ? 1 : 0);

  return (
    <div className="panel-footer">
      <div className="footer-info">
        {t('显示第 {start} 到 {end} 条，共 {total} 条', {
          start: total === 0 ? 0 : startIdx + 1,
          end: Math.min(startIdx + pageSize, total),
          total
        })}
      </div>
      {showPageButtons && (
        <div className="pagination">
          <button type="button" className="page-btn" aria-label={t('上页')} disabled={page === 1} onClick={() => onPageChange(Math.max(1, page - 1))}>
            &lt;
          </button>
          {Array.from({ length: totalPages }, (_, i) => i + 1).map(item => (
            <button type="button" key={item} className={`page-btn ${page === item ? 'active' : ''}`} aria-current={page === item ? 'page' : undefined} onClick={() => onPageChange(item)}>
              {item}
            </button>
          ))}
          <button type="button" className="page-btn" aria-label={t('下页')} disabled={page === totalPages} onClick={() => onPageChange(Math.min(totalPages, page + 1))}>
            &gt;
          </button>
        </div>
      )}
    </div>
  );
}
