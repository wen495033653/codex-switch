import { useState } from 'react';

const emptyIdeReopenModal = {
  visible: false,
  loading: false,
  snapshotId: '',
  summary: []
};

export function useIdeReopen({
  handleRes,
  requireStore,
  setStore,
  toast,
  toastError
}) {
  const [ideReopenModal, setIdeReopenModal] = useState(emptyIdeReopenModal);

  const showIdeReopen = (reopenInfo) => {
    if (!reopenInfo || !reopenInfo.snapshot_id) return;
    setIdeReopenModal({
      visible: true,
      loading: false,
      snapshotId: reopenInfo.snapshot_id,
      summary: Array.isArray(reopenInfo.summary) ? reopenInfo.summary : []
    });
  };

  const cancelIdeReopen = async () => {
    const snapshotId = ideReopenModal.snapshotId;
    setIdeReopenModal(emptyIdeReopenModal);
    if (snapshotId) {
      try {
        const res = await window.api.discardIdeSnapshot(snapshotId);
        if (res && res.store) setStore(requireStore(res));
        toast('账号已切换，稍后重启 Codex 或 VS Code 后生效');
      } catch {
        return;
      }
    }
  };

  const confirmIdeReopen = async () => {
    if (!ideReopenModal.snapshotId) return;
    setIdeReopenModal(prev => ({ ...prev, loading: true }));
    try {
      const res = await window.api.restartOpenIdes(ideReopenModal.snapshotId);
      handleRes(res);
      setIdeReopenModal(emptyIdeReopenModal);
    } catch (err) {
      toastError(err, '重启编辑器失败');
      setIdeReopenModal(prev => ({ ...prev, loading: false }));
    }
  };

  const ideSummaryText = ideReopenModal.summary.length > 0
    ? ideReopenModal.summary.map(item => (item.count > 1 ? `${item.displayName} x${item.count}` : item.displayName)).join('、')
    : '当前已打开的 Codex 或 VS Code';

  return {
    cancelIdeReopen,
    confirmIdeReopen,
    ideReopenModal,
    ideSummaryText,
    showIdeReopen
  };
}
