import { useMemo, useState } from 'react';
import { getAccountId, getAccountName, maskAccountDisplayName } from '../utils/auth';
import { getErrorMessage } from '../utils/errors';

const EMPTY_DELETE_ACCOUNT_MODAL = {
  visible: false,
  loading: false,
  accountId: '',
  accountName: ''
};

export function useAccountOperations({
  handleRes,
  maskAccountName,
  setStore,
  toast,
  toastError
}) {
  const [refreshingAccountId, setRefreshingAccountId] = useState('');
  const [deleteAccountModal, setDeleteAccountModal] = useState(EMPTY_DELETE_ACCOUNT_MODAL);

  const deleteAccountDisplayName = useMemo(() => (
    maskAccountName
      ? maskAccountDisplayName(deleteAccountModal.accountName)
      : deleteAccountModal.accountName
  ), [deleteAccountModal.accountName, maskAccountName]);

  const handleRefreshAccount = async (accountId) => {
    if (!accountId || refreshingAccountId === accountId) return;
    setRefreshingAccountId(accountId);
    try {
      const res = await window.api.refreshAccount(accountId);
      if (res && res.store) setStore(res.store);
      toast(res && res.message ? res.message : '配额已刷新');
    } catch (err) {
      toastError(err, '刷新配额失败');
    } finally {
      setRefreshingAccountId(prev => (prev === accountId ? '' : prev));
    }
  };

  const openDeleteAccountModal = (account) => {
    const accountId = getAccountId(account);
    if (!accountId) return;
    setDeleteAccountModal({
      visible: true,
      loading: false,
      accountId,
      accountName: getAccountName(account)
    });
  };

  const closeDeleteAccountModal = () => {
    if (deleteAccountModal.loading) return;
    setDeleteAccountModal(EMPTY_DELETE_ACCOUNT_MODAL);
  };

  const confirmDeleteAccount = async () => {
    if (!deleteAccountModal.accountId || deleteAccountModal.loading) return;
    setDeleteAccountModal(prev => ({ ...prev, loading: true }));
    try {
      const res = await window.api.deleteAccount(deleteAccountModal.accountId);
      handleRes(res);
      setDeleteAccountModal(EMPTY_DELETE_ACCOUNT_MODAL);
    } catch (err) {
      toastError(err, '删除账号失败');
      setDeleteAccountModal(prev => ({ ...prev, loading: false }));
    }
  };

  const exportAccountsToBackup = async () => {
    try {
      const res = await window.api.exportAccounts();
      handleRes(res);
    } catch (err) {
      const message = getErrorMessage(err, '导出失败');
      if (message !== '导出已取消') toast(message, 7000);
    }
  };

  return {
    deleteAccountDisplayName,
    deleteAccountModal,
    refreshingAccountId,
    closeDeleteAccountModal,
    confirmDeleteAccount,
    exportAccountsToBackup,
    handleRefreshAccount,
    openDeleteAccountModal
  };
}
