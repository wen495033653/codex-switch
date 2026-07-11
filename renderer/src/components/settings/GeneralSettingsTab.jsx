import {
    AUTO_START_OPTIONS,
    DEFAULT_UI_THEME,
    UI_LANGUAGE_OPTIONS,
    UI_THEME_OPTIONS
} from './options';
import { DEFAULT_UI_LANGUAGE, useI18n } from '../../i18n';

export default function GeneralSettingsTab({
    dataDir,
    isDevBuild = false,
    openDataDir,
    settingsDraft,
    updateSettingsDraftAndSave
}) {
    const { t } = useI18n();

    return (
        <>
            <section className="settings-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">{t('开机启动')}</div>
                </div>

                <div className="settings-option-list settings-option-list-inline">
                    {AUTO_START_OPTIONS.map(option => {
                        const active = (settingsDraft.auto_start === true) === option.value;
                        const disabled = isDevBuild && option.value === true;
                        return (
                            <button
                                key={String(option.value)}
                                type="button"
                                className={`settings-option ${active ? 'active' : ''}`}
                                disabled={disabled}
                                title={disabled ? t('开发模式不支持开机自启') : undefined}
                                onClick={() => updateSettingsDraftAndSave({
                                    auto_start: option.value,
                                    auto_start_launch_mode: 'tray'
                                })}
                            >
                                <span className="settings-option-radio" aria-hidden="true">
                                    <span className="settings-option-dot" />
                                </span>
                                <span className="settings-option-text">
                                    <span className="settings-option-title">{t(option.title)}</span>
                                    <span className="settings-option-desc">
                                        {disabled ? t('开发模式不支持开机自启，请使用安装后的正式版本。') : t(option.desc)}
                                    </span>
                                </span>
                            </button>
                        );
                    })}
                </div>
            </section>

            <section className="settings-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">{t('界面主题')}</div>
                </div>

                <div className="settings-option-list settings-option-list-inline settings-theme-options">
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
                                    <span className="settings-option-title">{t(option.title)}</span>
                                    <span className="settings-option-desc">{t(option.desc)}</span>
                                </span>
                            </button>
                        );
                    })}
                </div>
            </section>

            <section className="settings-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">{t('界面语言')}</div>
                </div>

                <div className="settings-option-list settings-option-list-inline settings-language-options">
                    {UI_LANGUAGE_OPTIONS.map(option => {
                        const active = (settingsDraft.ui_language || DEFAULT_UI_LANGUAGE) === option.value;
                        return (
                            <button
                                key={option.value}
                                type="button"
                                className={`settings-option ${active ? 'active' : ''}`}
                                onClick={() => updateSettingsDraftAndSave({ ui_language: option.value })}
                            >
                                <span className="settings-option-radio" aria-hidden="true">
                                    <span className="settings-option-dot" />
                                </span>
                                <span className="settings-option-text">
                                    <span className="settings-option-title">{t(option.title)}</span>
                                    <span className="settings-option-desc">{t(option.desc)}</span>
                                </span>
                            </button>
                        );
                    })}
                </div>
            </section>

            <section className="settings-section">
                <div className="settings-section-head">
                    <div className="settings-section-title">{t('数据目录')}</div>
                </div>

                <div className="settings-path-card">
                    <strong className="settings-path-value" title={dataDir}>{dataDir || '--'}</strong>
                    <button
                        type="button"
                        className="btn btn-secondary"
                        onClick={openDataDir}
                    >
                        {t('打开')}
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
                        <span className="settings-toggle-title">{t('自动检查更新')}</span>
                        <span className="settings-toggle-desc">{t('启动时自动检查新版本')}</span>
                    </span>
                    <span className="settings-switch" aria-hidden="true">
                        <span className="settings-switch-thumb" />
                    </span>
                </button>
            </section>
        </>
    );
}
