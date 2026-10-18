import { useI18n } from '../i18n';
import IntelligencePanel, { IntelligenceStartButton } from './IntelligencePanel';
import Modal from './Modal';

export default function SubscriptionIntelligenceModal({
  accountName,
  model,
  modelDraft,
  onClose,
  onModelDraftChange,
  onStart,
  onToggleTest,
  run,
  running,
  selectedTestIds
}) {
  const { t } = useI18n();
  return (
    <Modal title={t('{name} · 降智测试', { name: accountName })} width="760px" onClose={onClose}>
      <label className="iq-model-field">
        <span>{t('测试模型')}</span>
        <input
          value={modelDraft}
          placeholder={model}
          spellCheck={false}
          disabled={running}
          onChange={event => onModelDraftChange(event.target.value)}
        />
        <small>{t('用这个账号的 Codex 额度发题，和 Codex 走同一个接口')}</small>
      </label>
      <IntelligencePanel
        model={model}
        onToggleTest={onToggleTest}
        run={run}
        running={running}
        selectedTestIds={selectedTestIds}
      />
      <div className="iq-actions">
        <IntelligenceStartButton
          onStart={onStart}
          run={run}
          running={running}
          selectedCount={selectedTestIds.length}
        />
        <button type="button" className="btn btn-secondary iq-close-button" onClick={onClose}>
          {t('关闭')}
        </button>
      </div>
    </Modal>
  );
}
