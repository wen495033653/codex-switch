import { useEffect, useState } from 'react';
import { AppDialogs, AppMainView, AppNavbar } from './components';
import {
  DEFAULT_CODEX_STATE,
  DEFAULT_SETTINGS,
  GPT_POOL_URL,
  getActiveApiProfile,
  normalizeBackgroundRefreshInterval,
  normalizeApiProfiles,
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
    const nextApiProfiles = normalizeApiProfiles(rawSettings.api_profiles, rawSettings.api_mode);
    const nextActiveApiProfile = getActiveApiProfile({
      ...rawSettings,
      api_profiles: nextApiProfiles
    });
    const nextSettings = {
      ...DEFAULT_SETTINGS,
      ...rawSettings,
      active_api_profile_id: nextActiveApiProfile.id,
      api_profiles: nextApiProfiles,
      api_mode: nextActiveApiProfile
    };
    setSettings(nextSettings);
    setSettingsDraft(nextSettings);
    setApiDraft(nextSettings.api_mode || DEFAULT_SETTINGS.api_mode);
    setApiProfiles(nextSettings.api_profiles || DEFAULT_SETTINGS.api_profiles);
    setActiveApiProfileId(nextSettings.active_api_profile_id || DEFAULT_SETTINGS.active_api_profile_id);
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
    activeApiProfileId,
    addApiProfile,
    apiProfileModal,
    apiProfileDeleteModal,
    apiProfiles,
    clearApiAutoSaveTimer,
    closeApiProfileModal,
    closeDeleteApiProfileModal,
    confirmDeleteApiProfile,
    editApiProfile,
    openDeleteApiProfileModal,
    saveApiProfileModal,
    savingApiProfile,
    setActiveApiProfileId,
    setApiDraft,
    setApiProfiles,
    updateApiProfileModalDraft
  } = useApiModeDraft({
    applySettings,
    toastError
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
    activeApiProfileId,
    apiProfiles,
    applySettings,
    clearApiAutoSaveTimer,
    handleRes,
    showIdeReopen,
    toastError
  });
  const apiProfileBusy = savingApiMode || savingApiProfile;

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
    openAddModal,
    refreshTokenInput,
    refreshTokenLoading,
    setRefreshTokenInput,
    setShowRefreshTokenPanel,
    showRefreshTokenPanel,
    startOauth
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
            codexSessionSyncEnabled,
            checkingUpdate,
            savingCodexSessionSync,
            savingCodexProxyEnv,
            savingProxySettings,
            onToggleCodexSessionSync: updateCodexSessionSyncEnabled,
            setCodexProxyEnvEnabled,
            updateSettingsDraftAndSave,
            normalizeBackgroundRefreshInterval,
            openDataDir,
            updateCodexProxySettings,
            openRepository,
            handleCheckUpdate
          }}
          apiModePageProps={{
            activeApiProfileId,
            apiProfiles,
            onAddApiProfile: addApiProfile,
            onDeleteApiProfile: openDeleteApiProfileModal,
            onEditApiProfile: editApiProfile,
            onOpenCodexConfigToml: openCodexConfigToml,
            onOpenGptPool: openGptPoolLanding,
            onSwitchToApiMode: switchToApiModeFromPage,
            savingApiMode: apiProfileBusy,
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
            onStartOauth: startOauth,
            onToggleRefreshTokenPanel: () => setShowRefreshTokenPanel(v => !v)
          }}
          apiProfile={{
            modal: apiProfileModal,
            deleteModal: apiProfileDeleteModal,
            saving: apiProfileBusy || switching,
            onClose: closeApiProfileModal,
            onCancelDelete: closeDeleteApiProfileModal,
            onConfirmDelete: confirmDeleteApiProfile,
            onSave: saveApiProfileModal,
            onUpdate: updateApiProfileModalDraft
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
