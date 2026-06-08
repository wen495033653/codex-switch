import { useState } from 'react';
import {
  buildApiProfilePayload,
  buildApiSettingsPayload,
  DEFAULT_SETTINGS,
  normalizeApiBaseUrlInput,
  normalizeApiProfiles,
  upsertApiProfile
} from '../utils/appState';
import { getErrorMessage } from '../utils/errors';

function createApiProfileId() {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return `api-${crypto.randomUUID().slice(0, 8)}`;
  }
  return `api-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function getProfileById(profiles, id) {
  return profiles.find(profile => profile.id === id) || profiles[0] || DEFAULT_SETTINGS.api_mode;
}

function createEmptyApiProfileModal() {
  return {
    visible: false,
    mode: 'add',
    profileId: '',
    draft: DEFAULT_SETTINGS.api_mode,
    error: ''
  };
}

function createEmptyApiProfileDeleteModal() {
  return {
    visible: false,
    profileId: '',
    profileName: '',
    loading: false
  };
}

export function useApiModeDraft({
  applySettings,
  toastError
}) {
  const [apiDraft, setApiDraft] = useState(DEFAULT_SETTINGS.api_mode);
  const [apiProfiles, setApiProfiles] = useState(DEFAULT_SETTINGS.api_profiles);
  const [activeApiProfileId, setActiveApiProfileId] = useState(DEFAULT_SETTINGS.active_api_profile_id);
  const [apiProfileModal, setApiProfileModal] = useState(createEmptyApiProfileModal);
  const [apiProfileDeleteModal, setApiProfileDeleteModal] = useState(createEmptyApiProfileDeleteModal);
  const [savingApiProfile, setSavingApiProfile] = useState(false);

  const clearApiAutoSaveTimer = () => {};

  const persistApiSettings = async ({ activeId, activeProfile, profiles }) => {
    const res = await window.api.updateSettings(buildApiSettingsPayload({
      activeId,
      activeProfile,
      profiles
    }));
    applySettings(res);
    return res;
  };

  const addApiProfile = () => {
    const nextProfile = buildApiProfilePayload({
      id: createApiProfileId(),
      name: `API ${apiProfiles.length + 1}`,
      base_url: '',
      api_key: ''
    });
    setApiProfileModal({
      visible: true,
      mode: 'add',
      profileId: nextProfile.id,
      draft: nextProfile,
      error: ''
    });
  };

  const editApiProfile = (id) => {
    const profiles = normalizeApiProfiles(apiProfiles, apiDraft);
    const profile = getProfileById(profiles, id);
    setApiProfileModal({
      visible: true,
      mode: 'edit',
      profileId: profile.id,
      draft: profile,
      error: ''
    });
  };

  const closeApiProfileModal = () => {
    setApiProfileModal(createEmptyApiProfileModal());
  };

  const updateApiProfileModalDraft = (patch) => {
    setApiProfileModal(prev => ({
      ...prev,
      error: '',
      draft: {
        ...(prev.draft || DEFAULT_SETTINGS.api_mode),
        ...patch,
        id: (prev.draft && prev.draft.id) || prev.profileId || activeApiProfileId
      }
    }));
  };

  const getValidatedApiProfile = () => {
    const draft = apiProfileModal.draft || {};
    const name = String(draft.name || '').trim();
    const baseUrl = String(draft.base_url || '').trim();
    const apiKey = String(draft.api_key || '').trim();
    const missing = [];

    if (!name) missing.push('名称');
    if (!baseUrl) missing.push('Base URL');
    if (!apiKey) missing.push('API Key');

    if (missing.length > 0) {
      setApiProfileModal(prev => ({
        ...prev,
        error: `请填写${missing.join('、')}`
      }));
      return null;
    }

    let normalizedBaseUrl = '';
    try {
      normalizedBaseUrl = normalizeApiBaseUrlInput(baseUrl);
    } catch (err) {
      setApiProfileModal(prev => ({
        ...prev,
        error: getErrorMessage(err, 'API Base URL 格式无效')
      }));
      return null;
    }

    return buildApiProfilePayload({
      ...draft,
      name,
      base_url: normalizedBaseUrl,
      api_key: apiKey
    }, apiProfileModal.profileId);
  };

  const saveApiProfileModal = async () => {
    if (!apiProfileModal.visible || savingApiProfile) return;

    const profile = getValidatedApiProfile();
    if (!profile) return;

    const profiles = normalizeApiProfiles(apiProfiles, apiDraft);
    const nextProfiles = upsertApiProfile(profiles, profile);
    const nextActive = profile.id === activeApiProfileId
      ? profile
      : getProfileById(nextProfiles, activeApiProfileId);

    setSavingApiProfile(true);
    try {
      await persistApiSettings({
        activeId: nextActive.id,
        activeProfile: nextActive,
        profiles: nextProfiles
      });
      closeApiProfileModal();
    } catch (err) {
      setApiProfileModal(prev => ({
        ...prev,
        error: getErrorMessage(err, 'API 配置保存失败')
      }));
    } finally {
      setSavingApiProfile(false);
    }
  };

  const removeApiProfile = async (id) => {
    const profiles = normalizeApiProfiles(apiProfiles, apiDraft);
    if (profiles.length <= 1 || id === activeApiProfileId) {
      closeDeleteApiProfileModal();
      return;
    }

    const nextProfiles = profiles.filter(profile => profile.id !== id);
    const nextActive = getProfileById(nextProfiles, activeApiProfileId);

    setApiProfileDeleteModal(prev => ({ ...prev, loading: true }));
    try {
      await persistApiSettings({
        activeId: nextActive.id,
        activeProfile: nextActive,
        profiles: nextProfiles
      });
      setApiProfileModal(prev => (
        prev.visible && prev.profileId === id
          ? createEmptyApiProfileModal()
          : prev
      ));
      closeDeleteApiProfileModal();
    } catch (err) {
      setApiProfileDeleteModal(prev => ({ ...prev, loading: false }));
      toastError(err, '删除 API 配置失败');
    }
  };

  const openDeleteApiProfileModal = (id) => {
    const profiles = normalizeApiProfiles(apiProfiles, apiDraft);
    const profile = getProfileById(profiles, id);
    if (!profile || profiles.length <= 1 || profile.id === activeApiProfileId) return;
    setApiProfileDeleteModal({
      visible: true,
      profileId: profile.id,
      profileName: profile.name || profile.id,
      loading: false
    });
  };

  const closeDeleteApiProfileModal = () => {
    setApiProfileDeleteModal(createEmptyApiProfileDeleteModal());
  };

  const confirmDeleteApiProfile = async () => {
    if (!apiProfileDeleteModal.visible || !apiProfileDeleteModal.profileId || apiProfileDeleteModal.loading) return;
    await removeApiProfile(apiProfileDeleteModal.profileId);
  };

  return {
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
  };
}
