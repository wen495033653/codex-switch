import { useEffect, useMemo, useState } from 'react';
import { getAccountId, getAccountName, parseAuthInfo } from '../utils/auth';
import { useGridPageSize } from './useGridPageSize';

// Lives in App, so it outlives the accounts grid: useGridPageSize re-measures the grid
// each time the accounts page mounts it again.
export function useAccountPagination({
  accounts,
  activeId,
  filter,
  search
}) {
  const { gridRef: accountGridRef, pageSize } = useGridPageSize();
  const [page, setPage] = useState(1);

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
