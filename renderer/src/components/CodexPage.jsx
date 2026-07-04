import ProxySettingsTab, { CodexProcessCard } from './settings/ProxySettingsTab';

export default function CodexPage({
    accounts,
    codexSessionSyncEnabled,
    codexRemoteControlPendingEnabled,
    maskAccountName,
    onCodexRemoteControlAutoDisabled,
    onOpenCodexConfigToml,
    restartingCodexApp,
    restartCurrentCodexAppNormal,
    savingCodexProxyEnv,
    savingCodexModelInstructions,
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
    updateCodexProxySettings,
    updateSettingsDraftAndSave
}) {
    const modelInstructionsEnabled = settingsDraft.codex_model_instructions_enabled !== false;

    return (
        <div className="settings-page codex-page">
            <div className="settings-page-panel codex-page-panel">
                <div className="settings-page-head">
                    <div className="settings-page-title">Codex</div>
                    <button
                        type="button"
                        className="btn btn-secondary codex-config-open-button"
                        onClick={onOpenCodexConfigToml}
                    >
                        打开 config.toml
                    </button>
                </div>

                <div className="settings-modal settings-page-content-split">
                    <CodexProcessCard
                        restartingCodexApp={restartingCodexApp}
                        restartCurrentCodexAppNormal={restartCurrentCodexAppNormal}
                    />

                    <section className="settings-section settings-app-card-section codex-model-instructions-section">
                        <div className="codex-model-instructions-head">
                            <div className="settings-section-head">
                                <div className="settings-section-title">gpt破限</div>
                            </div>
                            <button
                                type="button"
                                className={`codex-model-instructions-switch ${modelInstructionsEnabled ? 'active' : ''}`}
                                aria-pressed={modelInstructionsEnabled}
                                aria-label={modelInstructionsEnabled ? '关闭 gpt破限' : '启动 gpt破限'}
                                disabled={switching || savingCodexModelInstructions}
                                onClick={() => setCodexModelInstructionsEnabled(!modelInstructionsEnabled)}
                            >
                                <span className="codex-model-instructions-switch-label">启动</span>
                                <span className="settings-switch" aria-hidden="true">
                                    <span className="settings-switch-thumb" />
                                </span>
                            </button>
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
                        onCodexRemoteControlAutoDisabled={onCodexRemoteControlAutoDisabled}
                        setSettingsDraft={setSettingsDraft}
                        setCodexProxyEnvEnabled={setCodexProxyEnvEnabled}
                        setCodexRemoteControlAccountId={setCodexRemoteControlAccountId}
                        setCodexRemoteControlEnabled={setCodexRemoteControlEnabled}
                        setCodexSessionSyncEnabled={setCodexSessionSyncEnabled}
                        settingsDraft={settingsDraft}
                        switching={switching}
                        updateCodexProxySettings={updateCodexProxySettings}
                        updateSettingsDraftAndSave={updateSettingsDraftAndSave}
                    />
                </div>
            </div>
        </div>
    );
}
