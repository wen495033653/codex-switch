import { useEffect, useRef, useState } from 'react';
import { getErrorMessage } from '../utils/errors';

const createUpdateModalState = () => ({
  visible: false,
  loading: false,
  status: 'idle',
  currentVersion: '',
  remoteVersion: '',
  notes: '',
  publishedAt: '',
  progress: 0,
  error: ''
});

export function useUpdateFlow({
  settings,
  settingsLoaded,
  toast,
  toastError
}) {
  const startupUpdateCheckedRef = useRef(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [updateModal, setUpdateModal] = useState(createUpdateModalState);

  const applyUpdateStatus = (data) => {
    if (!data || typeof data !== 'object') return;
    if (data.status === 'downloading') {
      const percent = data.progress && Number.isFinite(Number(data.progress.percent))
        ? Math.max(0, Math.min(100, Number(data.progress.percent)))
        : 0;
      setUpdateModal(prev => ({
        ...prev,
        visible: true,
        loading: true,
        status: 'downloading',
        progress: percent,
        error: ''
      }));
      return;
    }
    if (data.status === 'downloaded') {
      const update = data.update || {};
      setUpdateModal(prev => ({
        ...prev,
        visible: true,
        loading: false,
        status: 'downloaded',
        remoteVersion: update.version || prev.remoteVersion,
        notes: update.release_notes || prev.notes,
        publishedAt: update.release_date || prev.publishedAt,
        progress: 100,
        error: ''
      }));
      return;
    }
    if (data.status === 'error') {
      const message = data.error || '更新失败';
      setUpdateModal(prev => ({
        ...prev,
        loading: false,
        status: 'error',
        error: message
      }));
      toast(message, 7000);
    }
  };

  const handleCheckUpdate = async (manual = true) => {
    if (checkingUpdate) return;
    setCheckingUpdate(true);
    try {
      const res = await window.api.checkUpdate({ manual });
      if (res && res.has_update) {
        if (!manual && res.suppressed) return;
        setUpdateModal({
          visible: true,
          loading: false,
          status: 'available',
          currentVersion: res.current_version || '',
          remoteVersion: res.remote_version || '',
          notes: res.release_notes || res.notes || '',
          publishedAt: res.release_date || res.published_at || '',
          progress: 0,
          error: ''
        });
      } else if (manual) {
        toast((res && res.message) || '当前已是最新版本');
      }
    } catch (err) {
      if (manual) toastError(err, '检查更新失败');
    } finally {
      setCheckingUpdate(false);
    }
  };

  const closeUpdateModal = () => {
    setUpdateModal(createUpdateModalState());
  };

  useEffect(() => {
    if (!settingsLoaded || startupUpdateCheckedRef.current) return;
    startupUpdateCheckedRef.current = true;
    if (settings.auto_check_updates !== false) {
      handleCheckUpdate(false);
    }
  }, [settingsLoaded, settings.auto_check_updates]);

  const cancelUpdateModal = async () => {
    const remoteVersion = updateModal.remoteVersion;
    closeUpdateModal();
    if (!remoteVersion) return;
    try {
      await window.api.dismissUpdateVersion(remoteVersion);
    } catch {
      return;
    }
  };

  const confirmUpdateAction = async () => {
    setUpdateModal(prev => ({ ...prev, loading: true }));
    try {
      if (updateModal.status === 'downloaded') {
        if (typeof window.api.installUpdate !== 'function') {
          throw new Error('更新接口未加载，请重启 Codex Switch');
        }
        await window.api.installUpdate();
        return;
      }
      if (typeof window.api.downloadUpdate !== 'function') {
        throw new Error('更新接口未加载，请重启 Codex Switch');
      }
      window.api.downloadUpdate().catch(err => {
        const message = getErrorMessage(err, '下载更新失败');
        toast(message, 7000);
        setUpdateModal(prev => ({
          ...prev,
          loading: false,
          status: 'error',
          error: message
        }));
      });
      setUpdateModal(prev => ({
        ...prev,
        loading: true,
        status: 'downloading',
        error: ''
      }));
      toast('已开始下载更新');
    } catch (err) {
      const message = getErrorMessage(err, '更新操作失败');
      toast(message);
      setUpdateModal(prev => ({
        ...prev,
        loading: false,
        status: 'error',
        error: message
      }));
    }
  };

  return {
    applyUpdateStatus,
    cancelUpdateModal,
    checkingUpdate,
    confirmUpdateAction,
    handleCheckUpdate,
    updateModal
  };
}
