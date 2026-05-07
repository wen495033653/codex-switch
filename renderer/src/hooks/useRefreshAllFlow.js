import { useEffect, useRef, useState } from 'react';
import { DEFAULT_REFRESH_ALL_STATUS } from '../utils/appState';

export function useRefreshAllFlow({
  toast,
  toastError
}) {
  const [refreshModal, setRefreshModal] = useState(false);
  const [refreshAllStarting, setRefreshAllStarting] = useState(false);
  const [refreshAllStatus, setRefreshAllStatus] = useState(DEFAULT_REFRESH_ALL_STATUS);
  const refreshAllRunningRef = useRef(false);

  useEffect(() => {
    if (
      refreshAllRunningRef.current
      && !refreshAllStatus.running
      && refreshAllStatus.source !== 'auto'
    ) {
      toast(refreshAllStatus.message || '配额后台刷新已完成');
    }
    refreshAllRunningRef.current = refreshAllStatus.running;
  }, [refreshAllStatus.running, refreshAllStatus.message]);

  const openRefreshAllModal = () => {
    if (refreshAllStatus.running) {
      toast(refreshAllStatus.message || '后台刷新进行中');
      return;
    }
    setRefreshModal(true);
  };

  const closeRefreshAllModal = () => {
    if (!refreshAllStarting) setRefreshModal(false);
  };

  const handleRefreshAll = async () => {
    if (refreshAllStarting) return;
    setRefreshAllStarting(true);
    try {
      const res = await window.api.refreshAllQuotas();
      if (res && res.status) setRefreshAllStatus(res.status);
      toast(res && res.message ? res.message : '已开始后台刷新配额');
      setRefreshModal(false);
    } catch (err) {
      toastError(err, '启动后台刷新失败');
    } finally {
      setRefreshAllStarting(false);
    }
  };

  return {
    closeRefreshAllModal,
    handleRefreshAll,
    openRefreshAllModal,
    refreshAllStarting,
    refreshAllStatus,
    refreshModal,
    setRefreshAllStatus
  };
}
