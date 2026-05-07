import { useState } from 'react';
import { getAccountId, getAccountName, maskAccountDisplayName } from '../utils/auth';
import { getErrorMessage } from '../utils/errors';

const emptyRefreshTokenModal = {
  visible: false,
  accountId: '',
  accountName: '',
  refreshToken: '',
  loading: false,
  error: ''
};

export function useRefreshTokenModal({
  maskAccountName,
  requireStore,
  setStore,
  toast,
  toastError
}) {
  const [refreshTokenModal, setRefreshTokenModal] = useState(emptyRefreshTokenModal);

  const openRefreshTokenModal = (account) => {
    const refreshToken = account && account.tokens && typeof account.tokens.refresh_token === 'string'
      ? account.tokens.refresh_token
      : '';

    if (!refreshToken) {
      toast('该账号没有 Refresh Token');
      return;
    }

    setRefreshTokenModal({
      visible: true,
      accountId: getAccountId(account),
      accountName: getAccountName(account),
      refreshToken,
      loading: false,
      error: ''
    });
  };

  const closeRefreshTokenModal = () => {
    setRefreshTokenModal(emptyRefreshTokenModal);
  };

  const copyRefreshToken = async () => {
    if (!refreshTokenModal.refreshToken) return;

    try {
      await window.api.copyText(refreshTokenModal.refreshToken);
      toast('Refresh Token 已复制', 5000);
    } catch (err) {
      toastError(err, '复制 Refresh Token 失败');
    }
  };

  const handleRefreshAccountToken = async () => {
    if (!refreshTokenModal.accountId || refreshTokenModal.loading) return;

    setRefreshTokenModal(prev => ({ ...prev, loading: true }));
    try {
      const res = await window.api.refreshAccountToken(refreshTokenModal.accountId);
      if (!res || res.ok !== true) {
        if (res && res.store) setStore(res.store);
        setRefreshTokenModal(prev => ({
          ...prev,
          loading: false,
          error: res && res.message ? res.message : '刷新 Refresh Token 失败'
        }));
        return;
      }

      setStore(requireStore(res));
      setRefreshTokenModal(prev => ({
        ...prev,
        refreshToken: typeof res.refresh_token === 'string' && res.refresh_token ? res.refresh_token : prev.refreshToken,
        loading: false,
        error: ''
      }));
      toast(res.message || 'Refresh Token 已刷新');
    } catch (err) {
      setRefreshTokenModal(prev => ({
        ...prev,
        loading: false,
        error: getErrorMessage(err, '刷新 Refresh Token 失败')
      }));
    }
  };

  const refreshTokenAccountName = maskAccountName
    ? maskAccountDisplayName(refreshTokenModal.accountName)
    : refreshTokenModal.accountName;

  return {
    closeRefreshTokenModal,
    copyRefreshToken,
    handleRefreshAccountToken,
    openRefreshTokenModal,
    refreshTokenAccountName,
    refreshTokenModal
  };
}
