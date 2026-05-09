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
export const DEFAULT_API_PROFILE_ID = 'default';

export const DEFAULT_API_PROFILE = {
  id: DEFAULT_API_PROFILE_ID,
  name: DEFAULT_API_NAME,
  base_url: '',
  api_key: ''
};

export const DEFAULT_SETTINGS = {
  auto_start: true,
  auto_check_updates: true,
  background_refresh_enabled: true,
  background_refresh_interval_minutes: DEFAULT_BACKGROUND_REFRESH_INTERVAL_MINUTES,
  codex_proxy_url: DEFAULT_CODEX_PROXY_URL,
  codex_proxy_env_enabled: false,
  codex_session_sync_enabled: true,
  codex_active_mode: '',
  mask_account_name: false,
  ui_theme: 'light',
  active_api_profile_id: DEFAULT_API_PROFILE_ID,
  api_profiles: [DEFAULT_API_PROFILE],
  api_mode: DEFAULT_API_PROFILE
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
    id: raw.id ? String(raw.id) : DEFAULT_API_PROFILE_ID,
    name: raw.name ? raw.name : DEFAULT_API_NAME,
    base_url: raw.base_url ? raw.base_url : '',
    api_key: raw.api_key ? raw.api_key : ''
  };
};

export const buildApiProfilePayload = (source, fallbackId = DEFAULT_API_PROFILE_ID) => {
  const raw = source && typeof source === 'object' ? source : {};
  const id = raw.id ? String(raw.id).trim() : String(fallbackId || DEFAULT_API_PROFILE_ID).trim();
  return {
    id: id || DEFAULT_API_PROFILE_ID,
    name: raw.name ? String(raw.name) : DEFAULT_API_NAME,
    base_url: raw.base_url ? String(raw.base_url) : '',
    api_key: raw.api_key ? String(raw.api_key) : ''
  };
};

export const normalizeApiProfiles = (profiles, activeProfile = DEFAULT_API_PROFILE) => {
  const source = Array.isArray(profiles) ? profiles : [];
  const seen = new Set();
  const normalized = source
    .map((profile, index) => buildApiProfilePayload(profile, index === 0 ? DEFAULT_API_PROFILE_ID : `api-${index}`))
    .filter(profile => {
      if (!profile.id || seen.has(profile.id)) return false;
      seen.add(profile.id);
      return true;
    });

  if (normalized.length > 0) return normalized;
  return [buildApiProfilePayload(activeProfile)];
};

export const getActiveApiProfile = (settings) => {
  const rawSettings = settings && typeof settings === 'object' ? settings : {};
  const profiles = normalizeApiProfiles(rawSettings.api_profiles, rawSettings.api_mode);
  const activeId = rawSettings.active_api_profile_id || (rawSettings.api_mode && rawSettings.api_mode.id) || profiles[0].id;
  return profiles.find(profile => profile.id === activeId) || profiles[0] || buildApiProfilePayload(rawSettings.api_mode);
};

export const upsertApiProfile = (profiles, profile) => {
  const nextProfile = buildApiProfilePayload(profile);
  const normalized = normalizeApiProfiles(profiles, nextProfile);
  const index = normalized.findIndex(item => item.id === nextProfile.id);
  if (index >= 0) {
    return normalized.map(item => (item.id === nextProfile.id ? nextProfile : item));
  }
  return [...normalized, nextProfile];
};

export const buildApiSettingsPayload = ({ activeId, activeProfile, profiles }) => {
  const profile = buildApiProfilePayload(activeProfile, activeId || DEFAULT_API_PROFILE_ID);
  const profileList = upsertApiProfile(profiles, profile);
  const resolvedActiveId = profile.id || activeId || DEFAULT_API_PROFILE_ID;
  return {
    active_api_profile_id: resolvedActiveId,
    api_profiles: profileList,
    api_mode: profile
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
