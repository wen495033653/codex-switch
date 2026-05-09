import { useEffect, useState } from 'react';
import { createEmptyOauthState } from '../utils/appState';
import { getErrorMessage } from '../utils/errors';

export function useAddAccountFlow({
  handleRes,
  toast,
  toastError
}) {
  const [addModal, setAddModal] = useState(false);
  const [oauth, setOauth] = useState(createEmptyOauthState);
  const [oauthCallbackUrl, setOauthCallbackUrl] = useState('');
  const [oauthCallbackSubmitting, setOauthCallbackSubmitting] = useState(false);
  const [refreshTokenInput, setRefreshTokenInput] = useState('');
  const [refreshTokenLoading, setRefreshTokenLoading] = useState(false);
  const [showRefreshTokenPanel, setShowRefreshTokenPanel] = useState(false);

  const closeAddModal = () => {
    setAddModal(false);
    setOauthCallbackUrl('');
    setOauthCallbackSubmitting(false);
    setRefreshTokenInput('');
    setRefreshTokenLoading(false);
    setShowRefreshTokenPanel(false);
  };

  useEffect(() => {
    if (!addModal || !oauth.success) return undefined;
    const timer = setTimeout(() => closeAddModal(), 300);
    return () => clearTimeout(timer);
  }, [addModal, oauth.success]);

  const applyOauthUpdate = (data) => {
    if (data && data.errorCode === 'OAUTH_CANCELED') {
      setOauth(createEmptyOauthState());
      return;
    }
    if (data && data.success && data.message) {
      toast(data.message);
    }
    if (data && data.running === false && data.error && data.errorCode !== 'OAUTH_CANCELED') {
      toast(data.error, 7000);
    }
    setOauth(prev => ({ ...prev, ...data }));
  };

  const cancelOauth = async ({ silent = false } = {}) => {
    try {
      const res = await window.api.cancelOauth();
      setOauth(createEmptyOauthState());
      if (!silent && res && res.canceled) {
        toast(res.message || '已取消 OAuth 登录');
      }
      return res;
    } catch (err) {
      const message = getErrorMessage(err, '取消 OAuth 登录失败');
      if (!silent) toast(message);
      throw err;
    }
  };

  const startOauth = async () => {
    setOauth(createEmptyOauthState());
    setOauthCallbackUrl('');
    setOauthCallbackSubmitting(false);
    setOauth({ running: true, url: '', success: false, error: '', errorCode: '', message: '' });
    try {
      await window.api.startOauth();
    } catch (err) {
      const message = getErrorMessage(err, 'OAuth 登录失败');
      const errorCode = message.includes('进行中')
        ? 'OAUTH_ALREADY_RUNNING'
        : (err && typeof err.code === 'string' ? err.code : '');
      if (errorCode === 'OAUTH_CANCELED') {
        setOauth(createEmptyOauthState());
        return;
      }
      setOauth(prev => ({
        ...prev,
        running: errorCode === 'OAUTH_ALREADY_RUNNING',
        success: false,
        url: errorCode === 'OAUTH_ALREADY_RUNNING' ? prev.url : '',
        error: message,
        errorCode
      }));
      toast(message);
    }
  };

  const openAddModal = () => {
    setAddModal(true);
    if (!oauth.running) setOauth(createEmptyOauthState());
    setRefreshTokenInput('');
    setRefreshTokenLoading(false);
    setShowRefreshTokenPanel(false);
  };

  const submitOauthCallbackUrl = async () => {
    const callbackUrl = oauthCallbackUrl.trim();
    if (!callbackUrl) {
      toast('请输入回调 URL');
      return;
    }

    setOauthCallbackSubmitting(true);
    try {
      const res = await window.api.submitOauthCallback(callbackUrl);
      if (res && res.message) toast(res.message, 5000);
      setOauthCallbackUrl('');
    } catch (err) {
      toastError(err, '提交回调 URL 失败', 7000);
    } finally {
      setOauthCallbackSubmitting(false);
    }
  };

  const copyOauthUrl = async () => {
    if (!oauth.url) return;
    try {
      await window.api.copyText(oauth.url);
      toast('OAuth 链接已复制', 5000);
    } catch (err) {
      toast(getErrorMessage(err, '复制 OAuth 链接失败'), 7000);
    }
  };

  const captureCurrentAccount = () => {
    window.api.captureCurrent()
      .then(handleRes)
      .catch(err => toast(getErrorMessage(err, '读取本机 auth.json 失败'), 7000));
    closeAddModal();
  };

  const importByRefreshToken = async () => {
    const refreshToken = refreshTokenInput.trim();
    if (!refreshToken) {
      toast('请输入 refresh_token');
      return;
    }

    setRefreshTokenLoading(true);
    try {
      const res = await window.api.importRefreshToken(refreshToken);
      handleRes(res);
      setRefreshTokenInput('');
      setShowRefreshTokenPanel(false);
      closeAddModal();
    } catch (err) {
      toastError(err, '导入失败');
    } finally {
      setRefreshTokenLoading(false);
    }
  };

  const importAccountsFromBackup = async () => {
    try {
      const res = await window.api.importAccounts();
      handleRes(res);
      closeAddModal();
    } catch (err) {
      const message = getErrorMessage(err, '导入失败');
      if (message !== '导入已取消') toast(message, 7000);
    }
  };

  return {
    addModal,
    applyOauthUpdate,
    cancelOauth,
    captureCurrentAccount,
    closeAddModal,
    copyOauthUrl,
    importAccountsFromBackup,
    importByRefreshToken,
    oauth,
    oauthCallbackSubmitting,
    oauthCallbackUrl,
    openAddModal,
    refreshTokenInput,
    refreshTokenLoading,
    setRefreshTokenInput,
    setShowRefreshTokenPanel,
    showRefreshTokenPanel,
    startOauth,
    submitOauthCallbackUrl,
    setOauthCallbackUrl
  };
}
