import { ACCOUNT_NAME_DISPLAY_OPTIONS, DEFAULT_BACKGROUND_REFRESH_INTERVAL_MINUTES } from './options';

export default function AccountSettingsTab({
    normalizeBackgroundRefreshInterval,
    setSettingsDraft,
    settingsDraft,
    updateSettingsDraftAndSave
}) {
    return (
        <>
            <section className="settings-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">账号显示</div>
                </div>

                <div className="settings-option-list settings-option-list-inline">
                    {ACCOUNT_NAME_DISPLAY_OPTIONS.map(option => {
                        const active = (settingsDraft.mask_account_name === true) === option.value;
                        return (
                            <button
                                key={String(option.value)}
                                type="button"
                                className={`settings-option ${active ? 'active' : ''}`}
                                onClick={() => updateSettingsDraftAndSave({ mask_account_name: option.value })}
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

            <section className="settings-section settings-refresh-section">
                <button
                    type="button"
                    className={`settings-toggle-row settings-toggle-row-with-icon ${settingsDraft.background_refresh_enabled !== false ? 'active' : ''}`}
                    aria-pressed={settingsDraft.background_refresh_enabled !== false}
                    onClick={() => updateSettingsDraftAndSave({ background_refresh_enabled: settingsDraft.background_refresh_enabled === false })}
                >
                    <span className="settings-toggle-leading-icon" aria-hidden="true">
                        <svg viewBox="0 0 24 24" fill="none">
                            <path d="M20 6v5h-5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                            <path d="M4 18v-5h5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                            <path d="M18.1 9A7 7 0 0 0 6.2 6.4L4 8.5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                            <path d="M5.9 15A7 7 0 0 0 17.8 17.6L20 15.5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                    </span>
                    <span className="settings-toggle-copy">
                        <span className="settings-toggle-title">定时刷新全部账号</span>
                        <span className="settings-toggle-desc">按间隔刷新所有账号的配额信息</span>
                    </span>
                    <span className="settings-switch" aria-hidden="true">
                        <span className="settings-switch-thumb" />
                    </span>
                </button>

                <div className="settings-inline-field-row">
                    <span className="settings-inline-field-label">刷新间隔（分钟）</span>
                    <input
                        className="settings-input settings-number-input"
                        type="number"
                        min="1"
                        max="1440"
                        step="1"
                        value={settingsDraft.background_refresh_interval_minutes ?? DEFAULT_BACKGROUND_REFRESH_INTERVAL_MINUTES}
                        onChange={e => setSettingsDraft(prev => ({
                            ...prev,
                            background_refresh_interval_minutes: e.target.value
                        }))}
                        onBlur={e => updateSettingsDraftAndSave({
                            background_refresh_interval_minutes: normalizeBackgroundRefreshInterval(e.target.value)
                        })}
                        onKeyDown={e => {
                            if (e.key === 'Enter') e.currentTarget.blur();
                        }}
                    />
                </div>
            </section>
        </>
    );
}
