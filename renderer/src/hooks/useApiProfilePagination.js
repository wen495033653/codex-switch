import { useEffect, useMemo, useRef, useState } from 'react';
import { getFallbackPageSize } from '../utils/appState';

function getApiProfileHeight(grid) {
  const styles = window.getComputedStyle(grid);
  return Number.parseFloat(styles.getPropertyValue('--account-card-height')) || 0;
}

export function useApiProfilePagination({
  activeId,
  profiles
}) {
  const apiProfileGridRef = useRef(null);
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
      const grid = apiProfileGridRef.current;
      if (!grid) return;

      const styles = window.getComputedStyle(grid);
      const templateColumns = String(styles.gridTemplateColumns || '').trim();
      const columns = Math.max(1, templateColumns ? templateColumns.split(/\s+/).filter(Boolean).length : 1);
      const rowGap = Number.parseFloat(styles.rowGap || styles.gap || '0') || 0;
      const cardHeight = getApiProfileHeight(grid);
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
    if (typeof ResizeObserver === 'function' && apiProfileGridRef.current) {
      observer = new ResizeObserver(() => updateGridPageMetrics());
      observer.observe(apiProfileGridRef.current);
    }
    window.addEventListener('resize', updateGridPageMetrics);

    return () => {
      window.cancelAnimationFrame(frameId);
      if (observer) observer.disconnect();
      window.removeEventListener('resize', updateGridPageMetrics);
    };
  }, [viewportHeight]);

  const sortedProfiles = useMemo(() => {
    const list = [...(Array.isArray(profiles) ? profiles : [])];
    list.sort((a, b) => {
      const aId = a && a.id ? a.id : '';
      const bId = b && b.id ? b.id : '';
      return aId === activeId ? -1 : bId === activeId ? 1 : 0;
    });
    return list;
  }, [activeId, profiles]);

  const total = sortedProfiles.length;
  const totalPages = Math.ceil(total / pageSize);
  const startIdx = (page - 1) * pageSize;
  const currentItems = sortedProfiles.slice(startIdx, startIdx + pageSize);

  useEffect(() => {
    if (totalPages === 0 && page !== 1) {
      setPage(1);
      return;
    }
    if (totalPages > 0 && page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

  return {
    apiProfileGridRef,
    currentItems,
    page,
    pageSize,
    setPage,
    startIdx,
    total,
    totalPages
  };
}
