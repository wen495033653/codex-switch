import { useEffect, useState } from 'react';
import { AppDialogs, AppMainView, AppNavbar, DevDiagnosticsPanel } from './components';
import {
  DEFAULT_CODEX_STATE,
  DEFAULT_SETTINGS,
  GPT_POOL_URL,
  getActiveApiProfile,
  normalizeApiProfiles,
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
  useDevDiagnostics,
  useIdeReopen,
  useModeSwitching,
  useRefreshAllFlow,
  useRefreshTokenModal,
  useSettingsActions,
  useToast,
  useUpdateFlow
} from './hooks';

const IS_DEV_BUILD = import.meta.env.DEV;

export default function App() {
  if (IS_DEV_BUILD && isDevLogWindow()) {
    return <DevLogWindow />;
  }

  return <MainApp />;
}

function isDevLogWindow() {
  if (typeof window === 'undefined') return false;
  if (window.__CODEX_SWITCH_WINDOW_LABEL === 'dev-log') return true;
  const tauriWindowLabel = window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
  if (tauriWindowLabel === 'dev-log') return true;
  return new URLSearchParams(window.location.search).get('window') === 'dev-log';
}

function DevLogWindow() {
  const devDiagnostics = useDevDiagnostics({ enabled: IS_DEV_BUILD });
  const hideDevLogWindow = async () => {
    if (window.api && typeof window.api.hideDevLogWindow === 'function') {
      try {
        await window.api.hideDevLogWindow();
        return;
      } catch (_err) {
        // Fall through to the browser close path in dev:renderer.
      }
    }
    window.close();
  };

  return (
    <div className="app dev-build dev-log-window">
      <DevDiagnosticsPanel
        entries={devDiagnostics.entries}
        errorCount={devDiagnostics.errorCount}
        isOpen
        onClear={devDiagnostics.clear}
        onToggle={hideDevLogWindow}
        warningCount={devDiagnostics.warningCount}
      />
    </div>
  );
}

function MainApp() {
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
  const devDiagnostics = useDevDiagnostics({ enabled: IS_DEV_BUILD });
  const { message, toast, toastError } = useToast();
  const openDevLogWindow = async () => {
    if (!window.api || typeof window.api.openDevLogWindow !== 'function') {
      toast('开发日志窗口需要在桌面应用中打开', 5000);
      return;
    }
    try {
      await window.api.openDevLogWindow();
    } catch (err) {
      toastError(err, '打开开发日志失败', 7000);
    }
  };
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
    setApiProfiles(nextSettings.api_profiles || DEFAULT_SETTINGS.api_profiles);
    setActiveApiProfileId(nextSettings.active_api_profile_id || DEFAULT_SETTINGS.active_api_profile_id);
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
    activeApiProfileId,
    addApiProfile,
    apiDraft,
    apiProfileDeleteModal,
    apiProfileModal,
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
    activeApiProfileId,
    apiDraft,
    apiProfiles,
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
    pluginRestartNotice,
    openSettingsPage,
    restartingCodexApp,
    restartCurrentCodexAppNormal,
    codexRemoteControlPendingEnabled,
    savingCodexProxyEnv,
    savingCodexRemoteControl,
    savingProxySettings,
    setCodexProxyEnvEnabled,
    setCodexRemoteControlAccountId,
    setCodexRemoteControlEnabled,
    updateCodexProxySettings,
    updateSettingsDraftAndSave
  } = useSettingsActions({
    applySettings,
    settings,
    settingsDraft,
    setSettingsDraft,
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

  useEffect(() => {
    if (!IS_DEV_BUILD) return undefined;
    const baseTitle = (document.title || 'Codex Switch').replace(/^\[DEV\]\s*/, '') || 'Codex Switch';
    document.title = `[DEV] ${baseTitle}`;
    document.documentElement.dataset.build = 'dev';
    return () => {
      document.title = baseTitle;
      delete document.documentElement.dataset.build;
    };
  }, []);

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

  const apiProfileBusy = savingApiMode || savingApiProfile;
  const saveApiTestResults = async (apiTestResults) => {
    const nextApiTestResults = apiTestResults && typeof apiTestResults === 'object'
      ? apiTestResults
      : {};
    setSettings(prev => ({ ...prev, api_test_results: nextApiTestResults }));
    setSettingsDraft(prev => ({ ...prev, api_test_results: nextApiTestResults }));
    try {
      const res = await window.api.updateSettings({ api_test_results: nextApiTestResults });
      applySettings(res);
    } catch (err) {
      toastError(err, '保存 API 检查结果失败', 7000);
    }
  };

  return (
    <div className={`app${IS_DEV_BUILD ? ' dev-build' : ''}`}>
      <AppNavbar
        apiModeActive={apiModeActive}
        currentModeDetail={currentModeDetail}
        currentModeLabel={currentModeLabel}
        devErrorCount={devDiagnostics.errorCount}
        devLogCount={devDiagnostics.totalCount}
        isDevBuild={IS_DEV_BUILD}
        onDevDiagnosticsToggle={openDevLogWindow}
        onAccountsClick={() => setViewMode('accounts')}
        onApiClick={() => setViewMode('api')}
        onSessionsClick={() => setViewMode('sessions')}
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
            accounts: store.accounts,
            checkingUpdate,
            codexSessionSyncEnabled,
            isDevBuild: IS_DEV_BUILD,
            maskAccountName,
            savingCodexProxyEnv,
            savingCodexRemoteControl,
            savingCodexSessionSync,
            savingProxySettings,
            restartingCodexApp,
            restartCurrentCodexAppNormal,
            codexRemoteControlPendingEnabled,
            setCodexProxyEnvEnabled,
            setCodexRemoteControlAccountId,
            setCodexRemoteControlEnabled,
            setCodexSessionSyncEnabled: updateCodexSessionSyncEnabled,
            switching,
            updateSettingsDraftAndSave,
            normalizeBackgroundRefreshInterval,
            openDataDir,
            updateCodexProxySettings,
            openRepository,
            handleCheckUpdate,
            onOpenGptPool: openGptPoolLanding
          }}
          apiModePageProps={{
            activeApiProfileId,
            apiModeActive,
            apiProfiles,
            apiTestResults: settings.api_test_results,
            onAddApiProfile: addApiProfile,
            onDeleteApiProfile: openDeleteApiProfileModal,
            onEditApiProfile: editApiProfile,
            onOpenCodexConfigToml: openCodexConfigToml,
            onSaveApiTestResults: saveApiTestResults,
            onSwitchToApiMode: switchToApiModeFromPage,
            savingApiMode: apiProfileBusy,
            switching
          }}
          sessionManagerPageProps={{
            toast,
            toastError
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
          pluginRestartNotice={pluginRestartNotice}
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
