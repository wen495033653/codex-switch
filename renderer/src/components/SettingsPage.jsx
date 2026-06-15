import AccountSettingsTab from './settings/AccountSettingsTab';
import AboutSettingsTab from './settings/AboutSettingsTab';
import GeneralSettingsTab from './settings/GeneralSettingsTab';
import ProxySettingsTab from './settings/ProxySettingsTab';
import { SETTINGS_TABS } from './settings/options';

export default function SettingsPage({
    settingsTab,
    setSettingsTab,
    settingsDraft,
    setSettingsDraft,
    dataDir,
    appVersion,
    accounts,
    checkingUpdate,
    codexSessionSyncEnabled,
    isDevBuild,
    maskAccountName,
    subscriptionModeActive,
    savingCodexProxyEnv,
    savingCodexRemoteControl,
    savingCodexSessionSync,
    savingProxySettings,
    restartingCodexApp,
    restartCurrentCodexAppNormal,
    codexRemoteControlPendingEnabled,
    setCodexProxyEnvEnabled,
    setCodexRemoteControlAccountId,
    setCodexRemoteControlEnabled,
    setCodexSessionSyncEnabled,
    switching,
    updateSettingsDraftAndSave,
    normalizeBackgroundRefreshInterval,
    openDataDir,
    updateCodexProxySettings,
    openRepository,
    handleCheckUpdate,
    onOpenGptPool
}) {
    return (
        <div className="settings-page">
            <div className="settings-page-panel">
                <div className="settings-page-head">
                    <div className="settings-page-title">设置</div>
                </div>

                <div className="settings-page-toolbar">
                    <div className="settings-tabs">
                        {SETTINGS_TABS.map(tab => (
                            <button
                                key={tab.key}
                                type="button"
                                className={`settings-tab ${settingsTab === tab.key ? 'active' : ''}`}
                                onClick={() => setSettingsTab(tab.key)}
                            >
                                {tab.label}
                            </button>
                        ))}
                    </div>
                </div>

                <div className={`settings-modal settings-page-content ${settingsTab === 'proxy' ? 'settings-page-content-split' : ''}`}>
                    {settingsTab === 'general' && (
                        <GeneralSettingsTab
                            dataDir={dataDir}
                            isDevBuild={isDevBuild}
                            openDataDir={openDataDir}
                            settingsDraft={settingsDraft}
                            updateSettingsDraftAndSave={updateSettingsDraftAndSave}
                        />
                    )}

                    {settingsTab === 'account' && (
                        <AccountSettingsTab
                            normalizeBackgroundRefreshInterval={normalizeBackgroundRefreshInterval}
                            setSettingsDraft={setSettingsDraft}
                            settingsDraft={settingsDraft}
                            updateSettingsDraftAndSave={updateSettingsDraftAndSave}
                        />
                    )}

                    {settingsTab === 'proxy' && (
                        <ProxySettingsTab
                            accounts={accounts}
                            codexSessionSyncEnabled={codexSessionSyncEnabled}
                            maskAccountName={maskAccountName}
                            subscriptionModeActive={subscriptionModeActive}
                            savingCodexProxyEnv={savingCodexProxyEnv}
                            savingCodexRemoteControl={savingCodexRemoteControl}
                            savingCodexSessionSync={savingCodexSessionSync}
                            savingProxySettings={savingProxySettings}
                            restartingCodexApp={restartingCodexApp}
                            restartCurrentCodexAppNormal={restartCurrentCodexAppNormal}
                            codexRemoteControlPendingEnabled={codexRemoteControlPendingEnabled}
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
                    )}

                    {settingsTab === 'about' && (
                        <AboutSettingsTab
                            appVersion={appVersion}
                            checkingUpdate={checkingUpdate}
                            handleCheckUpdate={handleCheckUpdate}
                            onOpenGptPool={onOpenGptPool}
                            openRepository={openRepository}
                        />
                    )}
                </div>
            </div>
        </div>
    );
}
