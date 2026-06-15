import { useState } from 'react';
import Modal from './Modal';

function getPrecheckState(precheck) {
  if (!precheck) return 'idle';
  if (precheck.loading) return 'loading';
  return precheck.ok ? 'success' : 'error';
}

function getPrecheckLabel(precheck) {
  const state = getPrecheckState(precheck);
  if (state === 'loading') return '预检中';
  if (state === 'success') return '可用';
  if (state === 'error') return '不可用';
  return '未预检';
}

export default function ApiProfileModal({
  modal,
  saving,
  onClose,
  onSave,
  onUpdate
}) {
  const [showApiKey, setShowApiKey] = useState(false);
  const draft = modal && modal.draft ? modal.draft : {};
  const isEdit = modal && modal.mode === 'edit';
  const precheck = modal && modal.precheck ? modal.precheck : null;
  const precheckState = getPrecheckState(precheck);
  const submitText = precheck && precheck.loading
    ? '预检中...'
    : (saving ? '保存中...' : '保存配置');
  const handleClose = () => {
    if (!saving) onClose();
  };

  const handleSubmit = (event) => {
    event.preventDefault();
    if (!saving) onSave();
  };

  return (
    <Modal title={isEdit ? '编辑 API 配置' : '新增 API 配置'} onClose={handleClose} width="560px">
      <form className="api-profile-modal" onSubmit={handleSubmit} noValidate>
        {modal.error && (
          <div className="api-profile-modal-error" role="alert">
            {modal.error}
          </div>
        )}

        <label className="api-profile-modal-field">
          <span className="api-mode-label">名称</span>
          <input
            className="api-mode-input"
            required
            aria-invalid={Boolean(modal.error && !String(draft.name || '').trim())}
            value={draft.name || ''}
            placeholder="例如 OpenAI"
            onChange={event => onUpdate({ name: event.target.value })}
          />
        </label>

        <label className="api-profile-modal-field">
          <span className="api-mode-label">Base URL</span>
          <input
            className="api-mode-input"
            required
            aria-invalid={Boolean(modal.error && !String(draft.base_url || '').trim())}
            value={draft.base_url || ''}
            placeholder="https://api.example.com/v1"
            onChange={event => onUpdate({ base_url: event.target.value })}
          />
        </label>

        <label className="api-profile-modal-field">
          <span className="api-mode-label">API Key</span>
          <span className="api-key-input-wrap">
            <input
              className="api-mode-input api-key-input"
              type={showApiKey ? 'text' : 'password'}
              required
              aria-invalid={Boolean(modal.error && !String(draft.api_key || '').trim())}
              value={draft.api_key || ''}
              placeholder="sk-..."
              onChange={event => onUpdate({ api_key: event.target.value })}
            />
            <button
              type="button"
              className={`api-key-eye-button ${showApiKey ? 'active' : ''}`}
              aria-label={showApiKey ? '隐藏 API Key' : '显示 API Key'}
              title={showApiKey ? '隐藏 API Key' : '显示 API Key'}
              onClick={() => setShowApiKey(value => !value)}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M12 5.5c4.22 0 7.56 2.36 9.5 6.5-1.94 4.14-5.28 6.5-9.5 6.5S4.44 16.14 2.5 12C4.44 7.86 7.78 5.5 12 5.5Zm0 2C8.78 7.5 6.17 9.08 4.73 12 6.17 14.92 8.78 16.5 12 16.5s5.83-1.58 7.27-4.5C17.83 9.08 15.22 7.5 12 7.5Zm0 2.25A2.25 2.25 0 1 1 12 14.25 2.25 2.25 0 0 1 12 9.75Z" />
              </svg>
            </button>
          </span>
        </label>

        {precheck && (
          <div className={`api-profile-modal-precheck ${precheckState}`}>
            <div className="api-profile-modal-precheck-head">
              <span>配置预检</span>
              <strong>{getPrecheckLabel(precheck)}</strong>
            </div>
            <div className="api-profile-modal-precheck-message">
              {precheck.message || getPrecheckLabel(precheck)}
            </div>
          </div>
        )}

        <div className="api-profile-modal-actions">
          <button
            type="button"
            className="btn btn-secondary api-profile-modal-button"
            onClick={onClose}
            disabled={saving}
          >
            取消
          </button>
          <button
            type="submit"
            className="btn btn-primary api-profile-modal-button"
            disabled={saving}
          >
            {submitText}
          </button>
        </div>
      </form>
    </Modal>
  );
}
