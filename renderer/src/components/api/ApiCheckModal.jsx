import { useI18n } from '../../i18n';
import Modal from '../Modal';
import ModelPicker from '../ModelPicker';
import IntelligencePanel, { IntelligenceStartButton } from '../IntelligencePanel';
import ApiTestDetailContent, { ApiTestResponseBlock } from './ApiTestDetail';

export default function ApiCheckModal({
  checkModalProfileId,
  checkTab,
  closeCheckModal,
  commitTestModelDraft,
  detailModelOptions,
  detailPlaceholderText,
  detailProfile,
  detailProfileName,
  detailTest,
  effectiveDetailModel,
  handleTestBaseUrl,
  intelligenceRun,
  intelligenceRunning,
  modelDropdownOpen,
  modelsDetailAvailable,
  modelsDetailLabel,
  modelsDetailsOpen,
  modelsStatus,
  runIntelligenceTests,
  selectedTestIds,
  selectTestModelDraft,
  setCheckTab,
  setModelDropdownOpen,
  setModelsDetailsOpen,
  toggleTestSelected,
  visibleDetailTest,
}) {
  const { t } = useI18n();
  return (
    <Modal
      title={detailProfileName || checkModalProfileId || t('API 预检')}
      width="760px"
      onClose={closeCheckModal}
    >
      <div className="api-check-controls">
        <div className="api-check-model-field">
          <span>{t('测试模型')}</span>
          <ModelPicker
            disabled={Boolean(detailTest && detailTest.loading)}
            onClose={() => {
              setModelDropdownOpen(false);
              commitTestModelDraft(checkModalProfileId);
            }}
            onSelect={model => {
              selectTestModelDraft(checkModalProfileId, model);
              setModelDropdownOpen(false);
            }}
            open={modelDropdownOpen}
            options={detailModelOptions}
            setOpen={setModelDropdownOpen}
            value={effectiveDetailModel}
          />
          <div className="api-check-model-meta">
            <div className={`api-check-model-status ${modelsStatus.state}`}>
              {modelsStatus.text}
            </div>
            {modelsDetailAvailable && (
              <button
                type="button"
                className={`api-check-model-tag ${modelsStatus.state} ${modelsDetailsOpen ? 'active' : ''}`}
                onClick={() => setModelsDetailsOpen(open => !open)}
                aria-expanded={modelsDetailsOpen}
              >
                {modelsDetailLabel}
              </button>
            )}
          </div>
        </div>
      </div>

      {modelsDetailsOpen && modelsDetailAvailable && (
        <div className="api-check-model-detail">
          <ApiTestResponseBlock title={t('/models 详情')} response={detailTest.modelsResponse} />
        </div>
      )}

      <div className="nav-tabs api-check-tabs" role="tablist" aria-label={t('检查类型')}>
        {[['precheck', '连通性预检'], ['intelligence', '降智测试']].map(([key, label]) => (
          <button
            key={key}
            type="button"
            role="tab"
            aria-selected={checkTab === key}
            className={`nav-item ${checkTab === key ? 'active' : ''}`}
            onClick={() => setCheckTab(key)}
          >
            {t(label)}
          </button>
        ))}
      </div>

      {checkTab === 'intelligence' ? (
        <IntelligencePanel
          model={effectiveDetailModel}
          onToggleTest={toggleTestSelected}
          run={intelligenceRun}
          running={intelligenceRunning}
          selectedTestIds={selectedTestIds}
        />
      ) : visibleDetailTest ? (
        <ApiTestDetailContent test={visibleDetailTest} />
      ) : (
        <div className="api-check-placeholder">{detailPlaceholderText}</div>
      )}

      <div className="api-test-detail-actions">
        {checkTab === 'intelligence' ? (
          <IntelligenceStartButton
            disabled={!detailProfile}
            onStart={() => runIntelligenceTests(detailProfile, checkModalProfileId, effectiveDetailModel)}
            run={intelligenceRun}
            running={intelligenceRunning}
            selectedCount={selectedTestIds.length}
          />
        ) : (
          <button
            type="button"
            className="btn btn-primary api-test-detail-retest-button"
            disabled={!detailProfile || Boolean(detailTest && detailTest.loading)}
            onClick={() => handleTestBaseUrl(
              detailProfile,
              checkModalProfileId,
              detailProfile.name || (detailTest && detailTest.profileName) || checkModalProfileId,
              effectiveDetailModel
            )}
          >
            {t('重新预检')}
          </button>
        )}
        <button
          type="button"
          className="btn btn-secondary api-test-detail-close-button"
          onClick={closeCheckModal}
        >
          {t('关闭')}
        </button>
      </div>
    </Modal>
  );
}
