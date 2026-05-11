import { useEffect, useState } from 'react';
import { AppDialogs, AppMainView, AppNavbar } from './components';
import {
  DEFAULT_CODEX_STATE,
  DEFAULT_SETTINGS,
  GPT_POOL_URL,
  normalizeBackgroundRefreshInterval,
  OAUTH_TIMEOUT_HINT
} from './utils/appState';
import {
  useAddAccountFlow,
  useAccountOperations,
  useAccountPagination,
  useAppBootstrap,
  useApiModeDraft,
  useCodexSessionSync,
  useCurrentModeSummary,
  useIdeReopen,
  useModeSwitching,
  useRefreshAllFlow,
  useRefreshTokenModal,
  useSettingsActions,
  useToast,
  useUpdateFlow
} from './hooks';

export default function App() {
  const [store, setStore] = useState({ accounts: [], active_id: '' });
  const [codexState, setCodexState] = useState(DEFAULT_CODEX_STATE);
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [settingsDraft, setSettingsDraft] = useState(DEFAULT_SETTINGS);
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState('ALL');
  const [viewMode, setViewMode] = useState('accounts');
  const [settingsTab, setSettingsTab] = useState('general');
  const [appVersion, setAppVersion] = useState('');
  const [dataDir, setDataDir] = useState('');
  const [gptPoolAutoConfigModal, setGptPoolAutoConfigModal] = useState({
    visible: false,
    loading: false
  });

  const [settingsLoaded, setSettingsLoaded] = useState(false);
  const { message, toast, toastError } = useToast();
  const {
    closeRefreshAllModal,
    handleRefreshAll,
    openRefreshAllModal,
    refreshAllStarting,
    refreshAllStatus,
    refreshModal,
    setRefreshAllStatus
  } = useRefreshAllFlow({
    toast,
    toastError
  });

  const requireStore = (res) => {
    if (res && res.ok !== true) {
      throw new Error(res.message || '请求失败');
    }
    if (res && res.codex_state) {
      setCodexState(res.codex_state);
    }
    return res.store;
  };

  const applySettings = (res) => {
    if (!res || res.ok !== true) {
      throw new Error((res && res.message) || '设置加载失败');
    }

    const rawSettings = res.settings && typeof res.settings === 'object'
      ? res.settings
      : {};
    const nextSettings = {
      ...DEFAULT_SETTINGS,
      ...rawSettings,
      api_mode: {
        ...DEFAULT_SETTINGS.api_mode,
        ...((rawSettings.api_mode && typeof rawSettings.api_mode === 'object') ? rawSettings.api_mode : {})
      }
    };
    setSettings(nextSettings);
    setSettingsDraft(nextSettings);
    setApiDraft(nextSettings.api_mode || DEFAULT_SETTINGS.api_mode);
    setSettingsLoaded(true);
    return nextSettings;
  };

  const handleRes = (res) => {
    const nextStore = requireStore(res);
    if (res && res.message) toast(res.message);
    setStore(nextStore);
    return res;
  };

  const {
    apiDraft,
    clearApiAutoSaveTimer,
    setApiDraft,
    updateApiPageDraft
  } = useApiModeDraft({
    applySettings,
    settings,
    toastError,
    viewMode
  });

  const {
    accountGridRef,
    counts,
    currentItems,
    page,
    pageSize,
    setPage,
    startIdx,
    total,
    totalPages
  } = useAccountPagination({
    accounts: store.accounts,
    activeId: store.active_id,
    filter,
    search
  });

  const {
    apiConfigComplete,
    apiModeActive,
    currentAccountId,
    currentModeDetail,
    currentModeLabel,
    maskAccountName,
    subscriptionModeActive
  } = useCurrentModeSummary({
    apiDraft,
    codexState,
    settings,
    store
  });

  const {
    applyUpdateStatus,
    cancelUpdateModal,
    checkingUpdate,
    confirmUpdateAction,
    handleCheckUpdate,
    updateModal
  } = useUpdateFlow({
    settings,
    settingsLoaded,
    toast,
    toastError
  });

  const {
    cancelIdeReopen,
    confirmIdeReopen,
    ideReopenModal,
    ideSummaryText,
    showIdeReopen
  } = useIdeReopen({
    handleRes,
    requireStore,
    setStore,
    toast,
    toastError
  });

  const {
    handleSwitchAccount,
    savingApiMode,
    switching,
    switchToApiModeFromPage
  } = useModeSwitching({
    apiDraft,
    applySettings,
    clearApiAutoSaveTimer,
    handleRes,
    showIdeReopen,
    toastError
  });

  const {
    closeRefreshTokenModal,
    copyRefreshToken,
    handleRefreshAccountToken,
    openRefreshTokenModal,
    refreshTokenAccountName,
    refreshTokenModal
  } = useRefreshTokenModal({
    maskAccountName,
    requireStore,
    setStore,
    toast,
    toastError
  });

  const {
    closeDeleteAccountModal,
    confirmDeleteAccount,
    deleteAccountDisplayName,
    deleteAccountModal,
    exportAccountsToBackup,
    handleRefreshAccount,
    openDeleteAccountModal,
    refreshingAccountId
  } = useAccountOperations({
    handleRes,
    maskAccountName,
    setStore,
    toast,
    toastError
  });

  const {
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
    setOauthCallbackUrl,
    setShowRefreshTokenPanel,
    showRefreshTokenPanel,
    startOauth,
    submitOauthCallbackUrl
  } = useAddAccountFlow({
    handleRes,
    toast,
    toastError
  });

  const {
    openCodexConfigToml,
    openDataDir,
    openRepository,
    openSettingsPage,
    savingCodexProxyEnv,
    savingProxySettings,
    setCodexProxyEnvEnabled,
    updateCodexProxySettings,
    updateSettingsDraftAndSave
  } = useSettingsActions({
    applySettings,
    settings,
    settingsDraft,
    setSettingsDraft,
    setSettingsTab,
    setViewMode,
    toast,
    toastError
  });

  useAppBootstrap({
    applyOauthUpdate,
    applySettings,
    applyUpdateStatus,
    requireStore,
    setAppVersion,
    setCodexState,
    setDataDir,
    setRefreshAllStatus,
    setStore,
    toastError
  });

  useEffect(() => {
    const nextTheme = (viewMode === 'settings' ? settingsDraft.ui_theme : settings.ui_theme) || DEFAULT_SETTINGS.ui_theme;
    document.documentElement.dataset.theme = nextTheme;
  }, [settings.ui_theme, settingsDraft.ui_theme, viewMode]);

  const openGptPoolLanding = async () => {
    try {
      await window.api.openExternalUrl(GPT_POOL_URL);
    } catch (err) {
      toastError(err, '打开 gpt-pool.com 失败');
    }
  };

  const openGptPoolAutoConfigModal = () => {
    setGptPoolAutoConfigModal(() => ({
      visible: true,
      loading: false
    }));
  };

  const cancelGptPoolAutoConfig = () => {
    setGptPoolAutoConfigModal(prev => (
      prev.loading
        ? prev
        : { visible: false, loading: false }
    ));
  };

  const confirmGptPoolAutoConfig = async () => {
    if (gptPoolAutoConfigModal.loading) return;
    setGptPoolAutoConfigModal({
      visible: true,
      loading: true
    });
    try {
      const res = await window.api.configureGptPoolApi();
      applySettings(res);
      toast((res && res.message) || 'GPT Pool API 已配置');
      setGptPoolAutoConfigModal({
        visible: false,
        loading: false
      });
    } catch (err) {
      toastError(err, '自动配置 GPT Pool API 失败', 9000);
      setGptPoolAutoConfigModal({
        visible: false,
        loading: false
      });
    }
  };

  const {
    codexSessionSyncEnabled,
    savingCodexSessionSync,
    updateCodexSessionSyncEnabled
  } = useCodexSessionSync({
    applySettings,
    settings,
    setSettingsDraft,
    toastError
  });

  return (
    <div className="app">
      <AppNavbar
        apiModeActive={apiModeActive}
        currentModeDetail={currentModeDetail}
        currentModeLabel={currentModeLabel}
        onAccountsClick={() => setViewMode('accounts')}
        onApiClick={() => setViewMode('api')}
        onSettingsClick={openSettingsPage}
        subscriptionModeActive={subscriptionModeActive}
        viewMode={viewMode}
      />

      <div className="main-content">
        <AppMainView
          viewMode={viewMode}
          settingsPageProps={{
            settingsTab,
            setSettingsTab,
            settingsDraft,
            setSettingsDraft,
            dataDir,
            appVersion,
            checkingUpdate,
            savingCodexProxyEnv,
            savingProxySettings,
            setCodexProxyEnvEnabled,
            updateSettingsDraftAndSave,
            normalizeBackgroundRefreshInterval,
            openDataDir,
            updateCodexProxySettings,
            openRepository,
            handleCheckUpdate
          }}
          apiModePageProps={{
            apiConfigComplete,
            apiDraft,
            codexSessionSyncEnabled,
            apiModeActive,
            onConfigureGptPoolApi: openGptPoolAutoConfigModal,
            onOpenCodexConfigToml: openCodexConfigToml,
            onOpenGptPool: openGptPoolLanding,
            onToggleCodexSessionSync: updateCodexSessionSyncEnabled,
            onSwitchToApiMode: switchToApiModeFromPage,
            onUpdateApiDraft: updateApiPageDraft,
            savingApiMode,
            savingCodexSessionSync,
            switching
          }}
          accountsPageProps={{
            accountGridRef,
            apiModeActive,
            counts,
            currentAccountId,
            currentItems,
            filter,
            maskAccountName,
            onAddAccount: openAddModal,
            onDeleteAccount: openDeleteAccountModal,
            onExportAccounts: exportAccountsToBackup,
            onFilterChange: setFilter,
            onPageChange: setPage,
            onRefreshAccount: handleRefreshAccount,
            onRefreshAllClick: openRefreshAllModal,
            onSearchChange: setSearch,
            onSwitchAccount: handleSwitchAccount,
            onViewRefreshToken: openRefreshTokenModal,
            page,
            pageSize,
            refreshAllStatus,
            refreshingAccountId,
            search,
            startIdx,
            switching,
            total,
            totalPages
          }}
        />

        <AppDialogs
          message={message}
          addAccount={{
            visible: addModal,
            oauth,
            oauthCallbackSubmitting,
            oauthCallbackUrl,
            oauthTimeoutHint: OAUTH_TIMEOUT_HINT,
            refreshTokenInput,
            refreshTokenLoading,
            showRefreshTokenPanel,
            onCancelOauth: () => cancelOauth(),
            onCaptureCurrent: captureCurrentAccount,
            onClose: closeAddModal,
            onCopyOauthUrl: copyOauthUrl,
            onImportAccountsFromBackup: importAccountsFromBackup,
            onImportByRefreshToken: importByRefreshToken,
            onRefreshTokenInputChange: setRefreshTokenInput,
            onOauthCallbackUrlChange: setOauthCallbackUrl,
            onStartOauth: startOauth,
            onSubmitOauthCallbackUrl: submitOauthCallbackUrl,
            onToggleRefreshTokenPanel: () => setShowRefreshTokenPanel(v => !v)
          }}
          refreshToken={{
            accountName: refreshTokenAccountName,
            modal: refreshTokenModal,
            onClose: closeRefreshTokenModal,
            onCopy: copyRefreshToken,
            onRefresh: handleRefreshAccountToken
          }}
          deleteAccount={{
            displayName: deleteAccountDisplayName,
            modal: deleteAccountModal,
            onCancel: closeDeleteAccountModal,
            onConfirm: confirmDeleteAccount
          }}
          gptPoolAutoConfig={{
            visible: gptPoolAutoConfigModal.visible,
            loading: gptPoolAutoConfigModal.loading,
            onCancel: cancelGptPoolAutoConfig,
            onConfirm: confirmGptPoolAutoConfig
          }}
          refreshAll={{
            visible: refreshModal,
            isLoading: refreshAllStarting,
            onCancel: closeRefreshAllModal,
            onConfirm: handleRefreshAll
          }}
          ideReopen={{
            modal: ideReopenModal,
            summaryText: ideSummaryText,
            onCancel: cancelIdeReopen,
            onConfirm: confirmIdeReopen
          }}
          update={{
            modal: updateModal,
            onCancel: cancelUpdateModal,
            onConfirm: confirmUpdateAction
          }}
        />
      </div>
    </div>
  );
}
