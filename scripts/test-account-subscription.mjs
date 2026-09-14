import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

let server;
let AccountCard;
let I18nProvider;
let parseAuthInfo;

before(async () => {
  server = await createServer({
    configFile: fileURLToPath(new URL('../vite.config.mjs', import.meta.url)),
    server: { middlewareMode: true, hmr: false, watch: null },
    appType: 'custom',
    logLevel: 'error'
  });
  ({ default: AccountCard } = await server.ssrLoadModule('/src/components/AccountCard.jsx'));
  ({ I18nProvider } = await server.ssrLoadModule('/src/i18n.jsx'));
  ({ parseAuthInfo } = await server.ssrLoadModule('/src/utils/auth/info.js'));
});

after(async () => { await server?.close(); });

function account({ claimPlan = 'pro', activeUntil = '2026-09-04T12:29:43+00:00', usagePlan = 'pro', resetCredits = { available_count: 3, applicable_available_count: 0 }, subscription = null } = {}) {
  const claims = {
    email: 'fixture@example.invalid',
    'https://api.openai.com/auth': {
      chatgpt_account_id: 'fixture-account',
      chatgpt_plan_type: claimPlan,
      chatgpt_subscription_active_until: activeUntil,
      organizations: []
    }
  };
  return {
    profile_id: 'fixture-profile',
    tokens: {
      account_id: 'fixture-account',
      id_token: ['fixture', Buffer.from(JSON.stringify(claims)).toString('base64url'), 'fixture']
        .join('.')
    },
    custom: {
      auth_status: 'active',
      usage_status: 'ok',
      subscription,
      usage_info: {
        rate_limit: {
          primary_window: { used_percent: 100, limit_window_seconds: 604800, reset_at: 1789805431 },
          secondary_window: null
        },
        plan_type: usagePlan,
        reset_credits: resetCredits,
        fetched_at: '2026-09-14T03:37:11Z'
      }
    }
  };
}

test('plan type prefers the fresh usage plan over the id_token claim', () => {
  assert.equal(parseAuthInfo(account({ claimPlan: 'plus', usagePlan: 'pro' })).planType, 'pro');
  assert.equal(parseAuthInfo(account({ claimPlan: 'plus', usagePlan: '', subscription: SUBSCRIPTION })).planType, 'pro');
  assert.equal(parseAuthInfo(account({ claimPlan: 'plus', usagePlan: 'prolite' })).planType, 'pro');
  assert.equal(parseAuthInfo(account({ claimPlan: 'plus', usagePlan: '' })).planType, 'plus');
  assert.equal(parseAuthInfo(account({ claimPlan: 'pro', usagePlan: 'free' })).showExpiresAt, false);
});

test('reset credits are exposed only when the usage payload reports a count', () => {
  assert.deepEqual(parseAuthInfo(account()).resetCredits, { availableCount: 3, applicableAvailableCount: 0 });
  assert.equal(parseAuthInfo(account({ resetCredits: null })).resetCredits, null);
  assert.equal(parseAuthInfo(account({ resetCredits: { applicable_available_count: 1 } })).resetCredits, null);
  assert.equal(parseAuthInfo({ type: 'api', api: { configured: true } }).resetCredits, null);
});

function renderCard(acc, language = 'zh-CN') {
  return renderToStaticMarkup(createElement(I18nProvider, { preference: language },
    createElement(AccountCard, {
      acc,
      isCurrent: false,
      refreshing: false,
      switching: false,
      usageStats: null,
      maskAccountName: false,
      onSwitch() {},
      onOpenCodexAppInstance() {},
      openingCodexAppTarget: '',
      runningCodexAppInstances: {},
      onRefresh() {},
      onDelete() {},
      onViewRefreshToken() {},
      onOpenUsageStatsDetail() {}
    })));
}

test('account card shows the available reset count and hides it at zero', () => {
  const html = renderCard(account());
  const badge = html.match(/<span class="reset-credits-badge"[^>]*>([^<]*)<\/span>/);
  assert.ok(badge, 'reset badge must render');
  assert.equal(badge[1], '可重置 3 次');

  const english = renderCard(account(), 'en');
  assert.match(english, /<span class="reset-credits-badge"[^>]*>3 resets available<\/span>/);

  assert.doesNotMatch(renderCard(account({ resetCredits: { available_count: 0 } })), /reset-credits-badge/);
  assert.doesNotMatch(renderCard(account({ resetCredits: null })), /reset-credits-badge/);
});

const SUBSCRIPTION = {
  active_until: '2026-10-10T13:30:31Z',
  plan_type: 'pro',
  will_renew: true,
  is_delinquent: false,
  fetched_at: '2026-09-14T07:00:00Z'
};

test('the renewal date comes from the subscription endpoint and falls back to the claim', () => {
  const withEndpoint = parseAuthInfo(account({ activeUntil: '2026-09-04T12:29:43+00:00', subscription: SUBSCRIPTION }));
  assert.equal(withEndpoint.expiresAt, '2026-10-10T13:30:31Z');
  assert.equal(withEndpoint.expiresAtStale, false);
  assert.deepEqual(withEndpoint.subscription, {
    activeUntil: '2026-10-10T13:30:31Z',
    planType: 'pro',
    willRenew: true,
    isDelinquent: false,
    fetchedAt: '2026-09-14T07:00:00Z'
  });

  const withoutEndpoint = parseAuthInfo(account({ activeUntil: '2026-09-04T12:29:43+00:00' }));
  assert.equal(withoutEndpoint.expiresAt, '2026-09-04T12:29:43+00:00');
  assert.equal(withoutEndpoint.subscription, null);

  assert.equal(parseAuthInfo(account({ subscription: { plan_type: 'pro' } })).subscription, null);
  assert.equal(parseAuthInfo(account({ subscription: { active_until: '  ' } })).subscription, null);
  assert.equal(parseAuthInfo({ type: 'api', api: { configured: true } }).subscription, null);
});

test('account card shows the endpoint renewal date and the renewal tooltip', () => {
  const renewing = renderCard(account({ subscription: SUBSCRIPTION }));
  assert.match(renewing, /<span class="expire-date" title="订阅到期日期，到期后自动续费">到期 2026\/10\/10<\/span>/);
  assert.doesNotMatch(renewing, /到期时间未同步|delinquent-badge/);

  const ending = renderCard(account({ subscription: { ...SUBSCRIPTION, will_renew: false } }));
  assert.match(ending, /title="订阅到期日期，到期后不再续费"/);

  const unknown = renderCard(account({ subscription: { active_until: '2026-10-10T13:30:31Z' } }));
  assert.match(unknown, /title="订阅到期日期"/);

  const english = renderCard(account({ subscription: SUBSCRIPTION }), 'en');
  assert.match(english, /title="Subscription renewal date; renews automatically"/);
});

test('a past-due subscription is flagged on the card', () => {
  const html = renderCard(account({ subscription: { ...SUBSCRIPTION, is_delinquent: true } }));
  assert.match(html, /<span class="delinquent-badge" title="[^"]*">欠费<\/span>/);
  assert.match(renderCard(account({ subscription: { ...SUBSCRIPTION, is_delinquent: true } }), 'en'), />Past due<\/span>/);

  assert.doesNotMatch(renderCard(account({ subscription: SUBSCRIPTION })), /delinquent-badge/);
  assert.doesNotMatch(renderCard(account({ subscription: { active_until: '2026-10-10T13:30:31Z' } })), /delinquent-badge/);
});

test('an expired claim without endpoint data is reported as stale, not as the expiry', () => {
  const stale = parseAuthInfo(account({ activeUntil: '2026-09-04T12:29:43+00:00', usagePlan: 'pro' }));
  assert.equal(stale.expiresAtStale, true);
  assert.equal(stale.expiresAt, '2026-09-04T12:29:43+00:00');
  assert.equal(parseAuthInfo(account({ activeUntil: '2099-01-01T00:00:00Z', usagePlan: 'pro' })).expiresAtStale, false);
  assert.equal(parseAuthInfo(account({ activeUntil: '2026-09-04T12:29:43+00:00', usagePlan: '' })).expiresAtStale, false);
  assert.equal(parseAuthInfo(account({ activeUntil: '', usagePlan: 'pro' })).expiresAtStale, false);
  assert.equal(parseAuthInfo(account({ activeUntil: '2026-09-04T12:29:43+00:00', subscription: SUBSCRIPTION })).expiresAtStale, false);
  const beforeExpiry = Date.parse('2026-09-01T00:00:00Z');
  assert.equal(parseAuthInfo(account({ activeUntil: '2026-09-04T12:29:43+00:00' }), beforeExpiry).expiresAtStale, false);

  const html = renderCard(account({ activeUntil: '2026-09-04T12:29:43+00:00', usagePlan: 'pro' }));
  assert.match(html, /<span class="expire-date expire-date-stale" title="[^"]*PRO[^"]*">到期时间未同步<\/span>/);
  assert.doesNotMatch(html, /到期 2026/);
  assert.match(renderCard(account({ activeUntil: '2099-01-01T00:00:00Z' })), /<span class="expire-date" title="订阅到期日期">到期 2099\/1\/1<\/span>/);
});

test('account card plan badge follows the usage plan type', () => {
  const html = renderCard(account({ claimPlan: 'plus', usagePlan: 'pro' }));
  assert.match(html, /<span class="plan-badge plan-pro">PRO<\/span>/);
  assert.doesNotMatch(html, /plan-plus/);
});
