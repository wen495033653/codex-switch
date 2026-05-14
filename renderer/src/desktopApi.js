import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

function isTauriRuntime() {
  return typeof window !== 'undefined'
    && (Boolean(window.__TAURI_INTERNALS__) || Boolean(window.__TAURI__));
}

function subscribe(eventName, handler) {
  let disposed = false;
  const unlistenPromise = listen(eventName, event => {
    if (!disposed) handler(event.payload);
  });

  return () => {
    disposed = true;
    unlistenPromise.then(unlisten => unlisten()).catch(() => {});
  };
}

function tauriInvoke(command, args) {
  return invoke(command, args || {});
}

const COMMAND_BINDINGS = {
  getStore: ['get_store'],
  getAppVersion: ['get_app_version'],
  getDataDir: ['get_data_dir'],
  openDataDir: ['open_data_dir'],
  getRefreshAllStatus: ['get_refresh_all_status'],
  getSettings: ['get_settings'],
  updateSettings: ['update_settings', patch => ({ patch })],
  captureCurrent: ['capture_current'],
  importRefreshToken: ['import_refresh_token', token => ({ token })],
  deleteAccount: ['delete_account', id => ({ id })],
  switchAccount: ['switch_account', id => ({ id })],
  switchApiMode: ['switch_api_mode'],
  getCurrentCodexAppProcesses: ['get_current_codex_app_processes'],
  restartCurrentCodexAppForPluginSetting: ['restart_current_codex_app_for_plugin_setting'],
  restartCurrentCodexAppNormal: ['restart_current_codex_app_normal'],
  setCodexProxyEnvEnabled: ['set_codex_proxy_env_enabled', payload => ({
    enabled: Boolean(payload && payload.enabled),
    proxyUrl: payload && payload.proxyUrl ? payload.proxyUrl : ''
  })],
  restartOpenIdes: ['restart_open_ides', snapshotId => ({ snapshotId })],
  discardIdeSnapshot: ['discard_ide_snapshot', snapshotId => ({ snapshotId })],
  importAccounts: ['import_accounts'],
  exportAccounts: ['export_accounts'],
  refreshAllQuotas: ['refresh_all_quotas'],
  refreshAccount: ['refresh_account', id => ({ id })],
  refreshAccountToken: ['refresh_account_token', id => ({ id })],
  copyText: ['copy_text', text => ({ text })],
  checkUpdate: ['check_update', options => ({ options })],
  downloadUpdate: ['download_update'],
  installUpdate: ['install_update'],
  dismissUpdateVersion: ['dismiss_update_version', version => ({ version })],
  configureGptPoolApi: ['configure_gpt_pool_api'],
  openExternalUrl: ['open_external_url', url => ({ url })],
  openCodexConfigToml: ['open_codex_config_toml'],
  listBrandVoiceFiles: ['list_brand_voice_files'],
  scanSessions: ['session_manager_scan', root => ({ root })],
  previewSession: ['session_manager_preview', payload => ({
    root: payload.root,
    relativePath: payload.relativePath
  })],
  previewDeletedSession: ['session_manager_preview_deleted', deleteId => ({ deleteId })],
  selectSessionRoot: ['session_manager_select_root'],
  selectSessionWorkdir: ['session_manager_select_workdir'],
  exportSessions: ['session_manager_export', payload => ({
    root: payload.root,
    relativePaths: payload.relativePaths
  })],
  importSessions: ['session_manager_import', root => ({ root })],
  deleteSessions: ['session_manager_delete', payload => ({
    root: payload.root,
    relativePaths: payload.relativePaths
  })],
  listDeletedSessions: ['session_manager_list_deleted'],
  restoreDeletedSessions: ['session_manager_restore_deleted', payload => ({
    root: payload.root,
    deleteIds: payload.deleteIds,
    conflictStrategy: payload.conflictStrategy
  })],
  purgeDeletedSessions: ['session_manager_purge_deleted', deleteIds => ({ deleteIds })],
  setSessionStatus: ['session_manager_set_status', payload => ({
    root: payload.root,
    relativePaths: payload.relativePaths,
    status: payload.status,
    conflictStrategy: payload.conflictStrategy
  })],
  updateSessionCwd: ['session_manager_update_cwd', payload => ({
    root: payload.root,
    relativePaths: payload.relativePaths,
    cwd: payload.cwd
  })],
  startOauth: ['oauth_start', payload => ({ payload })],
  cancelOauth: ['oauth_cancel'],
  submitOauthCallback: ['oauth_submit_callback', callbackUrl => ({ callbackUrl })]
};

const EVENT_BINDINGS = {
  onOauthUpdate: 'oauth-update',
  onStoreUpdated: 'store-updated',
  onRefreshAllStatus: 'refresh-all-status',
  onUpdateStatus: 'update-status'
};

function unsupportedApiCall() {
  return Promise.reject(new Error('Tauri API 未加载，请使用 npm run dev 或桌面应用运行。'));
}

function noopSubscribe() {
  return () => {};
}

function installUnsupportedApiBridge() {
  window.api = {
    isTauriRuntime: false,
    ...Object.fromEntries(Object.keys(COMMAND_BINDINGS).map(name => [name, unsupportedApiCall])),
    ...Object.fromEntries(Object.keys(EVENT_BINDINGS).map(name => [name, noopSubscribe]))
  };
}

function installDesktopApiBridge() {
  window.api = {
    isTauriRuntime: true,
    ...Object.fromEntries(Object.entries(COMMAND_BINDINGS).map(([name, [command, buildArgs]]) => [
      name,
      (...args) => tauriInvoke(command, buildArgs ? buildArgs(...args) : undefined)
    ])),
    ...Object.fromEntries(Object.entries(EVENT_BINDINGS).map(([name, eventName]) => [
      name,
      handler => subscribe(eventName, handler)
    ]))
  };
}

export function installTauriApiBridge() {
  if (typeof window === 'undefined' || window.api) return;

  if (!isTauriRuntime()) {
    installUnsupportedApiBridge();
    return;
  }

  installDesktopApiBridge();
}
