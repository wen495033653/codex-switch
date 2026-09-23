import { useEffect, useMemo, useState } from 'react';
import { useGridPageSize } from './useGridPageSize';

export function useApiProfilePagination({
  activeId,
  profiles
}) {
  const { gridRef: apiProfileGridRef, pageSize } = useGridPageSize();
  const [page, setPage] = useState(1);

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
