export const DEFAULT_BACKGROUND_REFRESH_INTERVAL_MINUTES = 30;
export const DEFAULT_UI_THEME = 'system';

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
        title: '开启',
        desc: '开机后自动启动并收起到托盘。'
    },
    {
        value: false,
        title: '禁止',
        desc: '不开机自动启动。'
    }
];

export const UI_THEME_OPTIONS = [
    {
        value: 'system',
        title: '跟随系统',
        desc: '根据系统主题自动切换。'
    },
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

export const UI_LANGUAGE_OPTIONS = [
    {
        value: 'zh-CN',
        title: '简体中文',
        desc: '使用简体中文界面。'
    },
    {
        value: 'en',
        title: 'English',
        desc: 'Use the English interface.'
    }
];

export const SETTINGS_TABS = [
    { key: 'general', label: '通用' },
    { key: 'account', label: '账号' },
    { key: 'about', label: '关于' }
];
