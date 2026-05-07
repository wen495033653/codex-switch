const JWT_PARSE_CACHE_LIMIT = 200;
const jwtParseCache = new Map();

export function parseJwt(token) {
    if (typeof token !== 'string' || !token.trim()) throw new Error('id_token 缺失');
    const parts = token.split('.');
    if (parts.length < 2) throw new Error('id_token 格式无效');
    const payloadPart = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    const padding = '='.repeat((4 - (payloadPart.length % 4)) % 4);
    const binary = atob(payloadPart + padding);
    const bytes = Uint8Array.from(binary, char => char.charCodeAt(0));
    const payload = new TextDecoder('utf-8').decode(bytes);
    return JSON.parse(payload);
}

export function safeParseJwt(token) {
    const cacheKey = typeof token === 'string' ? token : '';
    if (jwtParseCache.has(cacheKey)) return jwtParseCache.get(cacheKey);

    const result = (() => {
        try {
            return {
                claims: parseJwt(token),
                error: ''
            };
        } catch (error) {
            return {
                claims: null,
                error: error && error.message ? error.message : 'id_token 解析失败'
            };
        }
    })();

    if (jwtParseCache.size >= JWT_PARSE_CACHE_LIMIT) jwtParseCache.clear();
    jwtParseCache.set(cacheKey, result);
    return result;
}
