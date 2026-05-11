import {
    AUTO_START_LAUNCH_OPTIONS,
    AUTO_START_OPTIONS,
    DEFAULT_AUTO_START_LAUNCH_MODE,
    DEFAULT_UI_THEME,
    UI_THEME_OPTIONS
} from './options';

export default function GeneralSettingsTab({
    dataDir,
    openDataDir,
    settingsDraft,
    updateSettingsDraftAndSave
}) {
    const autoStartEnabled = settingsDraft.auto_start === true;
    const autoStartLaunchMode = settingsDraft.auto_start_launch_mode || DEFAULT_AUTO_START_LAUNCH_MODE;

    return (
        <>
            <section className="settings-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">开机自启动</div>
                </div>

                <div className="settings-option-list settings-option-list-inline">
                    {AUTO_START_OPTIONS.map(option => {
                        const active = (settingsDraft.auto_start === true) === option.value;
                        return (
                            <button
                                key={String(option.value)}
                                type="button"
                                className={`settings-option ${active ? 'active' : ''}`}
                                onClick={() => updateSettingsDraftAndSave({ auto_start: option.value })}
                            >
                                <span className="settings-option-radio" aria-hidden="true">
                                    <span className="settings-option-dot" />
                                </span>
                                <span className="settings-option-text">
                                    <span className="settings-option-title">{option.title}</span>
                                </span>
                            </button>
                        );
                    })}
                </div>

                {autoStartEnabled ? (
                    <div className="settings-suboption-panel">
                        <div className="settings-suboption-title">启动后</div>
                        <div className="settings-option-list settings-option-list-inline settings-option-list-compact">
                            {AUTO_START_LAUNCH_OPTIONS.map(option => {
                                const active = autoStartLaunchMode === option.value;
                                return (
                                    <button
                                        key={option.value}
                                        type="button"
                                        className={`settings-option ${active ? 'active' : ''}`}
                                        onClick={() => updateSettingsDraftAndSave({ auto_start_launch_mode: option.value })}
                                    >
                                        <span className="settings-option-radio" aria-hidden="true">
                                            <span className="settings-option-dot" />
                                        </span>
                                        <span className="settings-option-text">
                                            <span className="settings-option-title">{option.title}</span>
                                        </span>
                                    </button>
                                );
                            })}
                        </div>
                    </div>
                ) : null}
            </section>

            <section className="settings-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">界面主题</div>
                </div>

                <div className="settings-option-list settings-option-list-inline">
                    {UI_THEME_OPTIONS.map(option => {
                        const active = (settingsDraft.ui_theme || DEFAULT_UI_THEME) === option.value;
                        return (
                            <button
                                key={option.value}
                                type="button"
                                className={`settings-option ${active ? 'active' : ''}`}
                                onClick={() => updateSettingsDraftAndSave({ ui_theme: option.value })}
                            >
                                <span className="settings-option-radio" aria-hidden="true">
                                    <span className="settings-option-dot" />
                                </span>
                                <span className="settings-option-text">
                                    <span className="settings-option-title">{option.title}</span>
                                    <span className="settings-option-desc">{option.desc}</span>
                                </span>
                            </button>
                        );
                    })}
                </div>
            </section>

            <section className="settings-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">数据目录</div>
                </div>

                <div className="settings-path-card">
                    <strong className="settings-path-value" title={dataDir}>{dataDir || '--'}</strong>
                    <button
                        type="button"
                        className="btn btn-secondary"
                        onClick={openDataDir}
                    >
                        打开
                    </button>
                </div>
            </section>

            <section className="settings-section">
                <button
                    type="button"
                    className={`settings-toggle-row ${settingsDraft.auto_check_updates !== false ? 'active' : ''}`}
                    aria-pressed={settingsDraft.auto_check_updates !== false}
                    onClick={() => updateSettingsDraftAndSave({ auto_check_updates: settingsDraft.auto_check_updates === false })}
                >
                    <span className="settings-toggle-copy">
                        <span className="settings-toggle-title">自动检查更新</span>
                        <span className="settings-toggle-desc">启动时自动检查新版本</span>
                    </span>
                    <span className="settings-switch" aria-hidden="true">
                        <span className="settings-switch-thumb" />
                    </span>
                </button>
            </section>
        </>
    );
}
