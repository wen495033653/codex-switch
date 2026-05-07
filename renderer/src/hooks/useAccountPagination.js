import { useEffect, useMemo, useRef, useState } from 'react';
import { getAccountId, getAccountName, parseAuthInfo } from '../utils/auth';
import { getFallbackPageSize } from '../utils/appState';

export function useAccountPagination({
  accounts,
  activeId,
  filter,
  search
}) {
  const accountGridRef = useRef(null);
  const [viewportHeight, setViewportHeight] = useState(() => window.innerHeight);
  const [page, setPage] = useState(1);
  const [gridPageMetrics, setGridPageMetrics] = useState({ columns: 0, rows: 0 });

  const pageSize = useMemo(() => {
    if (gridPageMetrics.columns > 0 && gridPageMetrics.rows > 0) {
      return gridPageMetrics.columns * gridPageMetrics.rows;
    }
    return getFallbackPageSize(viewportHeight);
  }, [gridPageMetrics, viewportHeight]);

  useEffect(() => {
    const handleResize = () => setViewportHeight(window.innerHeight);
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, []);

  useEffect(() => {
    const updateGridPageMetrics = () => {
      const grid = accountGridRef.current;
      if (!grid) return;

      const styles = window.getComputedStyle(grid);
      const templateColumns = String(styles.gridTemplateColumns || '').trim();
      const columns = Math.max(1, templateColumns ? templateColumns.split(/\s+/).filter(Boolean).length : 1);
      const rowGap = Number.parseFloat(styles.rowGap || styles.gap || '0') || 0;
      const cardHeight = Number.parseFloat(styles.getPropertyValue('--account-card-height')) || 0;
      if (!cardHeight) return;

      const rows = Math.max(1, Math.floor((grid.clientHeight + rowGap) / (cardHeight + rowGap)));
      setGridPageMetrics(prev => (
        prev.columns === columns && prev.rows === rows
          ? prev
          : { columns, rows }
      ));
    };

    const frameId = window.requestAnimationFrame(updateGridPageMetrics);
    let observer = null;
    if (typeof ResizeObserver === 'function' && accountGridRef.current) {
      observer = new ResizeObserver(() => updateGridPageMetrics());
      observer.observe(accountGridRef.current);
    }
    window.addEventListener('resize', updateGridPageMetrics);

    return () => {
      window.cancelAnimationFrame(frameId);
      if (observer) observer.disconnect();
      window.removeEventListener('resize', updateGridPageMetrics);
    };
  }, [viewportHeight]);

  useEffect(() => setPage(1), [search, filter]);

  const allItems = useMemo(() => {
    let list = [...accounts];
    if (search) {
      const normalizedSearch = search.toLowerCase();
      list = list.filter(account => getAccountName(account).toLowerCase().includes(normalizedSearch));
    }
    if (filter !== 'ALL') {
      list = list.filter(account => parseAuthInfo(account).planType.toUpperCase() === filter);
    }
    list.sort((a, b) => {
      const aId = getAccountId(a);
      const bId = getAccountId(b);
      return aId === activeId ? -1 : bId === activeId ? 1 : 0;
    });
    return list;
  }, [accounts, activeId, search, filter]);

  const total = allItems.length;
  const totalPages = Math.ceil(total / pageSize);
  const startIdx = (page - 1) * pageSize;
  const currentItems = allItems.slice(startIdx, startIdx + pageSize);

  useEffect(() => {
    if (totalPages === 0 && page !== 1) {
      setPage(1);
      return;
    }
    if (totalPages > 0 && page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

  const counts = useMemo(() => {
    const nextCounts = { ALL: accounts.length, FREE: 0, PLUS: 0, TEAM: 0, PRO: 0 };
    accounts.forEach(account => {
      const type = parseAuthInfo(account).planType.toUpperCase();
      if (nextCounts[type] !== undefined) nextCounts[type] += 1;
    });
    return nextCounts;
  }, [accounts]);

  return {
    accountGridRef,
    counts,
    currentItems,
    page,
    pageSize,
    setPage,
    startIdx,
    total,
    totalPages
  };
}
