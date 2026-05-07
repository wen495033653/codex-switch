export const DEFAULT_BACKGROUND_REFRESH_INTERVAL_MINUTES = 30;
export const DEFAULT_UI_THEME = 'light';

export const ACCOUNT_NAME_DISPLAY_OPTIONS = [
    {
        value: false,
        title: '显示完整账号',
        desc: '账号卡片正常显示邮箱或账号名称。'
    },
    {
        value: true,
        title: '脱敏显示',
        desc: '账号卡片和账号弹窗保留少量前缀，其余用 * 隐藏。'
    }
];

export const AUTO_START_OPTIONS = [
    {
        value: true,
        title: '开启'
    },
    {
        value: false,
        title: '禁止'
    }
];

export const UI_THEME_OPTIONS = [
    {
        value: 'dark',
        title: '暗黑模式',
        desc: '使用当前深色界面。'
    },
    {
        value: 'light',
        title: '白色模式',
        desc: '切换为浅色界面。'
    }
];

export const SETTINGS_TABS = [
    { key: 'general', label: '通用' },
    { key: 'account', label: '账号' },
    { key: 'proxy', label: 'Codex app' },
    { key: 'about', label: '关于' }
];
