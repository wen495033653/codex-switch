import { useEffect, useState } from 'react';
import { getFallbackPageSize, getGridPageMetrics } from '../utils/appState';

function measureGrid(grid) {
  const styles = window.getComputedStyle(grid);
  return getGridPageMetrics({
    templateColumns: styles.gridTemplateColumns,
    rowGap: Number.parseFloat(styles.rowGap || styles.gap || '0') || 0,
    cardHeight: Number.parseFloat(styles.getPropertyValue('--account-card-height')) || 0,
    clientHeight: grid.clientHeight
  });
}

// Page size of a card grid: as many cards as fit. `gridRef` is a callback ref, so a grid that
// unmounts with its page and mounts again later is measured and observed again.
export function useGridPageSize() {
  const [grid, setGrid] = useState(null);
  const [viewportHeight, setViewportHeight] = useState(() => window.innerHeight);
  const [gridPageMetrics, setGridPageMetrics] = useState({ columns: 0, rows: 0 });

  useEffect(() => {
    const handleResize = () => setViewportHeight(window.innerHeight);
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, []);

  useEffect(() => {
    if (!grid) return undefined;

    const updateGridPageMetrics = () => {
      const next = measureGrid(grid);
      if (!next) return;
      setGridPageMetrics(prev => (
        prev.columns === next.columns && prev.rows === next.rows ? prev : next
      ));
    };

    const frameId = window.requestAnimationFrame(updateGridPageMetrics);
    const observer = typeof ResizeObserver === 'function'
      ? new ResizeObserver(() => updateGridPageMetrics())
      : null;
    if (observer) observer.observe(grid);
    window.addEventListener('resize', updateGridPageMetrics);

    return () => {
      window.cancelAnimationFrame(frameId);
      if (observer) observer.disconnect();
      window.removeEventListener('resize', updateGridPageMetrics);
    };
  }, [grid]);

  const pageSize = gridPageMetrics.columns > 0 && gridPageMetrics.rows > 0
    ? gridPageMetrics.columns * gridPageMetrics.rows
    : getFallbackPageSize(viewportHeight);

  return { gridRef: setGrid, pageSize };
}
