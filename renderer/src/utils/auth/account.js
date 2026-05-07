export const API_MODE_ACCOUNT_ID = '__codex_api_mode__';

export function isApiModeAccount(account) {
    return account && account.type === 'api';
}

export function getAccountId(account) {
    if (isApiModeAccount(account)) return API_MODE_ACCOUNT_ID;
    const accountId = account && account.tokens ? account.tokens.account_id : '';
    return accountId;
}
