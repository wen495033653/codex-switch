import { useState } from 'react';
import AccountSettingsTab from './settings/AccountSettingsTab';
import RuntimeLogDialog from './settings/RuntimeLogDialog';
import AboutSettingsTab from './settings/AboutSettingsTab';
import GeneralSettingsTab from './settings/GeneralSettingsTab';
import { SETTINGS_TABS } from './settings/options';
import { useI18n } from '../i18n';

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
    const { t } = useI18n();
    const [logsVisible, setLogsVisible] = useState(false);
    const activeSettingsTab = SETTINGS_TABS.some(tab => tab.key === settingsTab)
        ? settingsTab
        : 'general';

    return (
        <div className="settings-page">
            <div className="settings-page-panel">
                <div className="settings-page-toolbar">
                    <div className="settings-tabs">
                        {SETTINGS_TABS.map(tab => (
                            <button
                                key={tab.key}
                                type="button"
                                className={`settings-tab ${activeSettingsTab === tab.key ? 'active' : ''}`}
                                onClick={() => setSettingsTab(tab.key)}
                            >
                                {t(tab.label)}
                            </button>
                        ))}
                    </div>
                    <button type="button" className="btn btn-secondary settings-log-button" onClick={() => setLogsVisible(true)}>
                        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true">
                            <rect x="5" y="3" width="14" height="18" rx="2" />
                            <path d="M9 8h6M9 12h6M9 16h4" />
                        </svg>
                        {t('日志')}
                    </button>
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
            {logsVisible && <RuntimeLogDialog onClose={() => setLogsVisible(false)} />}
        </div>
    );
}
