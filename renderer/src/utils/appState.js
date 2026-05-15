export const createEmptyOauthState = () => ({
  running: false,
  url: '',
  success: false,
  error: '',
  errorCode: '',
  message: ''
});

export const DEFAULT_REFRESH_ALL_STATUS = {
  running: false,
  total: 0,
  completed: 0,
  updated: 0,
  failed: 0,
  started_at: '',
  finished_at: '',
  message: ''
};

export const OAUTH_TIMEOUT_HINT = '等待浏览器授权，5 分钟未完成会自动取消。';
export const DEFAULT_BACKGROUND_REFRESH_INTERVAL_MINUTES = 30;
export const DEFAULT_CODEX_PROXY_URL = '127.0.0.1:10808';
export const DEFAULT_API_NAME = 'api';

export const DEFAULT_SETTINGS = {
  auto_start: true,
  auto_start_launch_mode: 'tray',
  auto_check_updates: true,
  background_refresh_enabled: true,
  background_refresh_interval_minutes: DEFAULT_BACKGROUND_REFRESH_INTERVAL_MINUTES,
  codex_proxy_url: DEFAULT_CODEX_PROXY_URL,
  codex_proxy_env_enabled: false,
  codex_plugins_enabled: false,
  codex_delete_button_enabled: false,
  codex_session_sync_enabled: true,
  codex_active_mode: '',
  api_promo_bar_open: false,
  mask_account_name: false,
  ui_theme: 'light',
  api_mode: {
    name: DEFAULT_API_NAME,
    base_url: '',
    api_key: ''
  }
};

export const DEFAULT_CODEX_STATE = {
  mode: 'unknown',
  auth_mode: '',
  preferred_auth_method: '',
  forced_login_method: '',
  model_provider: '',
  provider_name: '',
  wire_api: '',
  supports_websockets: false,
  openai_base_url: '',
  api_key_present: false,
  api_provider_ready: false,
  account_id: ''
};

export const REPOSITORY_URL = 'https://github.com/wen495033653/codex-switch';
export const GPT_POOL_URL = 'https://gpt-pool.com';
export const API_PROMO_CONFIG_URL = 'https://raw.githubusercontent.com/wen495033653/codex-switch/main/renderer/public/ad-config.json';

export const getFallbackPageSize = (viewportHeight) => {
  if (viewportHeight < 660) return 4;
  if (viewportHeight < 820) return 6;
  return 9;
};

export const normalizeBackgroundRefreshInterval = (value) => {
  const number = Number(value);
  if (!Number.isFinite(number)) return DEFAULT_BACKGROUND_REFRESH_INTERVAL_MINUTES;
  return Math.max(1, Math.min(1440, Math.round(number)));
};

export const buildApiModePayload = (source) => {
  const raw = source && typeof source === 'object' ? source : {};
  return {
    name: raw.name ? raw.name : DEFAULT_API_NAME,
    base_url: raw.base_url ? raw.base_url : '',
    api_key: raw.api_key ? raw.api_key : ''
  };
};

export const getApiProviderDisplayName = (source) => {
  const raw = source && typeof source === 'object' ? source : {};
  const name = String(raw.name || '').trim();
  if (name && name !== DEFAULT_API_NAME) return name;

  const baseUrl = String(raw.base_url || '').trim();
  if (!baseUrl) return '';
  const value = baseUrl.includes('://') ? baseUrl : `https://${baseUrl}`;
  try {
    return new URL(value).hostname || '';
  } catch (_err) {
    return '';
  }
};
