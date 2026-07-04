import AccountSettingsTab from './settings/AccountSettingsTab';
import AboutSettingsTab from './settings/AboutSettingsTab';
import GeneralSettingsTab from './settings/GeneralSettingsTab';
import { SETTINGS_TABS } from './settings/options';

export default function SettingsPage({
    settingsTab,
    setSettingsTab,
    settingsDraft,
    setSettingsDraft,
    dataDir,
    appVersion,
    checkingUpdate,
    isDevBuild,
    updateSettingsDraftAndSave,
    normalizeBackgroundRefreshInterval,
    openDataDir,
    openRepository,
    handleCheckUpdate,
    onOpenGptPool
}) {
    const activeSettingsTab = SETTINGS_TABS.some(tab => tab.key === settingsTab)
        ? settingsTab
        : 'general';

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
                                className={`settings-tab ${activeSettingsTab === tab.key ? 'active' : ''}`}
                                onClick={() => setSettingsTab(tab.key)}
                            >
                                {tab.label}
                            </button>
                        ))}
                    </div>
                </div>

                <div className="settings-modal settings-page-content">
                    {activeSettingsTab === 'general' && (
                        <GeneralSettingsTab
                            dataDir={dataDir}
                            isDevBuild={isDevBuild}
                            openDataDir={openDataDir}
                            settingsDraft={settingsDraft}
                            updateSettingsDraftAndSave={updateSettingsDraftAndSave}
                        />
                    )}

                    {activeSettingsTab === 'account' && (
                        <AccountSettingsTab
                            normalizeBackgroundRefreshInterval={normalizeBackgroundRefreshInterval}
                            setSettingsDraft={setSettingsDraft}
                            settingsDraft={settingsDraft}
                            updateSettingsDraftAndSave={updateSettingsDraftAndSave}
                        />
                    )}

                    {activeSettingsTab === 'about' && (
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
