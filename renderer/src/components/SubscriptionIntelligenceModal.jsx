import { useI18n } from '../i18n';
import IntelligencePanel, { IntelligenceStartButton } from './IntelligencePanel';
import Modal from './Modal';
import ModelPicker from './ModelPicker';

function modelListStatus(modelList, t, translateRuntimeText) {
  if (modelList.loading) return { state: 'loading', text: t('正在获取模型列表'), title: '' };
  if (modelList.error) {
    return { state: 'error', text: t('模型列表获取失败'), title: translateRuntimeText(modelList.error) };
  }
  return {
    state: 'success',
    text: t('{count} 个可用模型（Codex {version}）', {
      count: modelList.models.length,
      version: modelList.clientVersion
    }),
    title: ''
  };
}

export default function SubscriptionIntelligenceModal({
  accountName,
  model,
  modelList,
  modelMenuOpen,
  onClose,
  onModelSelect,
  onStart,
  onToggleTest,
  run,
  running,
  selectedTestIds,
  setModelMenuOpen
}) {
  const { t, translateRuntimeText } = useI18n();
  const status = modelListStatus(modelList, t, translateRuntimeText);
  const options = modelList.models.includes(model) ? modelList.models : [model, ...modelList.models];
  return (
    <Modal title={t('{name} · 降智测试', { name: accountName })} width="760px" onClose={onClose}>
      <div className="api-check-controls">
        <div className="api-check-model-field">
          <span>{t('测试模型')}</span>
          <ModelPicker
            disabled={running}
            onClose={() => setModelMenuOpen(false)}
            onSelect={selected => {
              onModelSelect(selected);
              setModelMenuOpen(false);
            }}
            open={modelMenuOpen}
            options={options}
            setOpen={setModelMenuOpen}
            value={model}
          />
          <div className="api-check-model-meta">
            <div className={`api-check-model-status ${status.state}`} title={status.title || undefined}>
              {status.text}
            </div>
          </div>
        </div>
      </div>
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
