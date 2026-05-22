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
    checkingUpdate,
    codexSessionSyncEnabled,
    savingCodexProxyEnv,
    savingCodexRemoteControlHook,
    savingCodexSessionSync,
    savingProxySettings,
    restartingCodexApp,
    restartCurrentCodexAppNormal,
    setCodexProxyEnvEnabled,
    setCodexRemoteControlHookEnabled,
    setCodexSessionSyncEnabled,
    switching,
    updateSettingsDraftAndSave,
    normalizeBackgroundRefreshInterval,
    openDataDir,
    updateCodexProxySettings,
    openRepository,
    handleCheckUpdate
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
                            codexSessionSyncEnabled={codexSessionSyncEnabled}
                            savingCodexProxyEnv={savingCodexProxyEnv}
                            savingCodexRemoteControlHook={savingCodexRemoteControlHook}
                            savingCodexSessionSync={savingCodexSessionSync}
                            savingProxySettings={savingProxySettings}
                            restartingCodexApp={restartingCodexApp}
                            restartCurrentCodexAppNormal={restartCurrentCodexAppNormal}
                            setSettingsDraft={setSettingsDraft}
                            setCodexProxyEnvEnabled={setCodexProxyEnvEnabled}
                            setCodexRemoteControlHookEnabled={setCodexRemoteControlHookEnabled}
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
                            openRepository={openRepository}
                        />
                    )}
                </div>
            </div>
        </div>
    );
}
