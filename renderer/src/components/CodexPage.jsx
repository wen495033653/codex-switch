import ProxySettingsTab, { CodexProcessCard } from './settings/ProxySettingsTab';
import { useCodexProcessStatus, useRemoteControlStatus } from '../hooks';
import { useI18n } from '../i18n';
import ModelInstructionsConfirmModal from './ModelInstructionsConfirmModal';

// Controller of the Codex page: it runs the page's status polls and hands the results to
// the view sections.

export default function CodexPage({
    accounts,
    codexSessionSyncEnabled,
    codexRemoteControlPendingEnabled,
    maskAccountName,
    onCodexRemoteControlAutoDisabled,
    onOpenCodexConfigToml,
    onOpenCodexDesktopUpdate,
    restartingCodexApp,
    restartCurrentCodexAppNormal,
    savingCodexProxyEnv,
    savingCodexModelInstructions,
    modelInstructionsConfirmation,
    savingCodexRemoteControl,
    savingCodexSessionSync,
    savingProxySettings,
    setCodexProxyEnvEnabled,
    setCodexModelInstructionsEnabled,
    setCodexRemoteControlAccountId,
    setCodexRemoteControlEnabled,
    setCodexSessionSyncEnabled,
    setSettingsDraft,
    settingsDraft,
    subscriptionModeActive,
    switching,
    updateCodexProxySettings
}) {
    const { t } = useI18n();
    const modelInstructionsEnabled = settingsDraft.codex_model_instructions_enabled !== false;
    const codexAppProcessStatus = useCodexProcessStatus();
    const remoteControlStatus = useRemoteControlStatus({
        enabled: settingsDraft.codex_remote_control_enabled === true,
        accountId: String(settingsDraft.codex_remote_control_account_id || '').trim(),
        onAutoDisabled: onCodexRemoteControlAutoDisabled
    });

    return (
        <div className="settings-page codex-page">
            {modelInstructionsConfirmation && <ModelInstructionsConfirmModal {...modelInstructionsConfirmation} />}
            <div className="settings-page-panel codex-page-panel">
                <div className="codex-page-actions">
                    <button
                        type="button"
                        className="btn btn-secondary codex-config-open-button"
                        onClick={onOpenCodexConfigToml}
                    >
                        {t('打开 config.toml')}
                    </button>
                </div>

                <div className="settings-modal settings-page-content-split">
                    <CodexProcessCard
                        codexAppProcessStatus={codexAppProcessStatus}
                        onOpenCodexDesktopUpdate={onOpenCodexDesktopUpdate}
                        restartingCodexApp={restartingCodexApp}
                        restartCurrentCodexAppNormal={restartCurrentCodexAppNormal}
                    />

                    <section className="settings-section settings-app-card-section codex-model-instructions-section">
                        <div className="settings-feature-head codex-model-instructions-head">
                            <div className="settings-section-head">
                                <div className="settings-section-title">{t('GPT 破甲')}</div>
                                <div className="settings-section-desc">{t('替换 instruction.md')}</div>
                            </div>
                            <div className="codex-model-instructions-actions">
                                {modelInstructionsEnabled && (
                                    <button
                                        type="button"
                                        className="codex-model-instructions-secondary-button"
                                        disabled={switching || savingCodexModelInstructions}
                                        onClick={() => setCodexModelInstructionsEnabled(false)}
                                    >
                                        {t('关闭')}
                                    </button>
                                )}
                                <button
                                    type="button"
                                    className="codex-model-instructions-primary-button"
                                    disabled={switching || savingCodexModelInstructions}
                                    onClick={() => setCodexModelInstructionsEnabled(true)}
                                >
                                    {savingCodexModelInstructions ? t('处理中...') : t('一键替换')}
                                </button>
                            </div>
                        </div>
                    </section>

                    <ProxySettingsTab
                        accounts={accounts}
                        codexSessionSyncEnabled={codexSessionSyncEnabled}
                        maskAccountName={maskAccountName}
                        subscriptionModeActive={subscriptionModeActive}
                        savingCodexProxyEnv={savingCodexProxyEnv}
                        savingCodexRemoteControl={savingCodexRemoteControl}
                        savingCodexSessionSync={savingCodexSessionSync}
                        savingProxySettings={savingProxySettings}
                        codexRemoteControlPendingEnabled={codexRemoteControlPendingEnabled}
                        remoteControlStatus={remoteControlStatus}
                        setSettingsDraft={setSettingsDraft}
                        setCodexProxyEnvEnabled={setCodexProxyEnvEnabled}
                        setCodexRemoteControlAccountId={setCodexRemoteControlAccountId}
                        setCodexRemoteControlEnabled={setCodexRemoteControlEnabled}
                        setCodexSessionSyncEnabled={setCodexSessionSyncEnabled}
                        settingsDraft={settingsDraft}
                        switching={switching}
                        updateCodexProxySettings={updateCodexProxySettings}
                    />
                </div>
            </div>
        </div>
    );
}
