import { useI18n } from '../../i18n';
import Modal from '../Modal';
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
          <div
            className="api-check-model-picker"
            onBlur={event => {
              if (!event.currentTarget.contains(event.relatedTarget)) {
                setModelDropdownOpen(false);
                commitTestModelDraft(checkModalProfileId);
              }
            }}
          >
            <button
              type="button"
              className="api-check-model-trigger"
              disabled={Boolean(detailTest && detailTest.loading)}
              onClick={() => setModelDropdownOpen(open => !open)}
              aria-haspopup="listbox"
              aria-expanded={modelDropdownOpen}
            >
              <span title={effectiveDetailModel}>{effectiveDetailModel}</span>
              <svg aria-hidden="true" viewBox="0 0 20 20" fill="currentColor">
                <path fillRule="evenodd" d="M5.23 7.21a.75.75 0 0 1 1.06.02L10 11.168l3.71-3.938a.75.75 0 1 1 1.08 1.04l-4.25 4.5a.75.75 0 0 1-1.08 0l-4.25-4.5a.75.75 0 0 1 .02-1.06Z" clipRule="evenodd" />
              </svg>
            </button>
            {modelDropdownOpen && !(detailTest && detailTest.loading) && (
              <div className="api-check-model-menu" role="listbox">
                {detailModelOptions.map(model => {
                  const selected = model === effectiveDetailModel;
                  return (
                    <button
                      key={model}
                      type="button"
                      className={`api-check-model-option ${selected ? 'selected' : ''}`}
                      role="option"
                      aria-selected={selected}
                      onClick={() => {
                        selectTestModelDraft(checkModalProfileId, model);
                        setModelDropdownOpen(false);
                      }}
                    >
                      {model}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
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
