import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const locatorSource = fs.readFileSync(path.join(root, 'src-tauri/src/codex_launcher/cdp/api_quota.js'), 'utf8');
const locate = vm.runInNewContext(locatorSource);
const bundleUrl = 'app://-/assets/app-primary-quota-fixture.js';
const bundle = `(() => {
  const staticDependencies=["./composer-content-quota-fixture.js"];
  const $={},_O={},_b={},cv={},yJe={};
  let configTracked=false;
  const Ud=(_,read)=>{let dirty=true,value;return{invalidate:()=>{dirty=true},read:()=>{if(dirty){value=read({get:e});dirty=false;}return value}}};
  function e(atom) { if(atom===_O)return {authMethod:"chatgpt"}; if(atom===_b)return {data:globalThis.fixtureState.quota}; if(atom===cv){configTracked=true;return {data:globalThis.fixtureState.config}}; throw Error("unexpected fixture atom"); }
  let Yz;
  Yz=Ud($,({get:e})=>{let t=e(_O),n=e(_b).data;if(t.authMethod!==\`chatgpt\`||n.rate_limit?.allowed!==!1)return!1;return!0});
  function providerCheck(e,t){let r=null,i="local";let{data:a}=e.get(cv,{cwd:r,hostId:i});return a==null||(e.get(yJe,t)??a.model_provider??\`openai\`)!==\`openai\`?null:true}
  globalThis.fixtureGate=()=>Yz.read();
  globalThis.fixtureSetConfig=config=>{const changed=JSON.stringify(config)!==JSON.stringify(fixtureState.config);fixtureState.config=config;if(changed&&configTracked)Yz.invalidate()};
})();
//# sourceURL=${bundleUrl}`;

const bannerUrl = 'app://-/assets/composer-content-quota-fixture.js';
const bannerBundle = `(() => {
 const Gr="local",Ol={},im={},ra={},eC={},ud={};
 const J=()=>({get:()=>({data:globalThis.fixtureState.config})});
 const q=atom=>atom===im?{data:globalThis.fixtureState.quota,dataUpdatedAt:0}:false;
 function AM(e){let t=0,{hostId:n,selectedModel:r}=e,i=n===void 0?Gr:n,a=J(Ol);let{data:g,dataUpdatedAt:_}=q(im),v=q(ra),y=q(eC),b=0;let L=null,z=null,ee=null,te=g?.rate_limit_upsell!=null,ne=fixtureGate();return{imageGenerationLimit:L,isUsageBannerEnabled:te,onDismissImageGenerationLimit:z,rateLimit:g,rateLimitWarningThreshold:ee,sendBlocked:ne}}
 function configHint(V,b,y){let{data:S}=V(ud,b),C=y??S?.model_provider??null;return C}
 globalThis.fixtureBanner=()=>AM({hostId:"local",selectedModel:"fixture"});
})();
//# sourceURL=${bannerUrl}`;

const mixedConfig = () => ({ model_provider: 'api', model_providers: { api: { requires_openai_auth: true, experimental_bearer_token: 'fixture-bearer' } } });

test('quota locator uniquely identifies the current selector and effective config descriptor', () => {
  const location = locate(bundle, bundleUrl);
  assert.equal(location.atom, 'Yz');
  assert.equal(location.getter, 'e');
  assert.equal(location.data, 'n');
  assert.equal(location.config, 'cv');
  assert.equal(bundle.split('\n')[location.lineNumber].slice(location.columnNumber, location.columnNumber + 3), 'if(');
  assert.match(location.condition, /provider === "api"/);
  assert.match(location.condition, /experimental_bearer_token/);
  assert.match(location.condition, /n = \{ \.\.\.n, rate_limit:/);
  assert.throws(() => locate('', bundleUrl), /selector count=0/);
  assert.throws(() => locate(bundle + bundle, bundleUrl), /selector count=2/);
  assert.throws(() => locate(bundle.replaceAll('rate_limit?.allowed', 'quota.allowed'), bundleUrl), /guard missing/);
  assert.throws(() => locate(bundle.replaceAll('model_provider', 'unknown_provider'), bundleUrl), /descriptor count=0/);
});

test('condition clones only local quota data and respects provider on every evaluation', () => {
  const condition = locate(bundle, bundleUrl).condition;
  const shared = Object.freeze({ rate_limit: Object.freeze({ allowed: false, limit_reached: true }) });
  const context = vm.createContext({ config: mixedConfig(), shared });
  vm.runInContext(`const cv={};const e=()=>({data:config});let n=shared;`, context);
  for (const [config, expected] of [[mixedConfig(), true], [{model_provider:'openai'}, false], [{model_provider:'api',model_providers:{api:{requires_openai_auth:true}}},false], [mixedConfig(), true]]) {
    context.config = config;
    vm.runInContext('n=shared;', context);
    assert.equal(vm.runInContext(condition, context), false);
    assert.equal(vm.runInContext('n.rate_limit.allowed', context), expected);
    assert.equal(shared.rate_limit.allowed, false);
    assert.equal(vm.runInContext('n.rate_limit.limit_reached', context), true);
  }
  vm.runInContext('n=shared;', context);
  context.config = undefined;
  vm.runInContext(condition, context);
  assert.equal(vm.runInContext('n.rate_limit.allowed', context), false);
});

test('only mixed remote-control quota hides the render-local official banner', () => {
  const location = locate(bannerBundle, bannerUrl);
  assert.equal(location.kind, 'official_quota_banner');
  assert.equal(location.data, 'g');
  const shared = Object.freeze({rate_limit:Object.freeze({allowed:false}),rate_limit_upsell:Object.freeze({banner_type:'usage_limit'})});
  const context = vm.createContext({config:mixedConfig(), shared});
  vm.runInContext('const ud={};const a={get:()=>({data:config})};const i="local";let g=shared;', context);
  for (const [config, hidden] of [[mixedConfig(),true],[{model_provider:'api',model_providers:{api:{requires_openai_auth:true}}},false],[{model_provider:'openai'},false]]) {
    context.config=config;
    vm.runInContext('g=shared', context);
    assert.equal(vm.runInContext(location.condition, context),false);
    assert.equal(vm.runInContext('g===null',context),hidden);
    assert.equal(shared.rate_limit.allowed,false);
    assert.equal(shared.rate_limit_upsell.banner_type,'usage_limit');
  }
});

function exitResult(child) {
  return new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolve({ code, signal }));
  });
}

async function runRustFixture(t, fixtureCase) {
  const fixtureCode = `
    import http from 'node:http';
    import inspector from 'node:inspector';
    const fixtureCase=${JSON.stringify(fixtureCase)};
    globalThis.fixtureState={config:${JSON.stringify(mixedConfig())},quota:Object.freeze({rate_limit:Object.freeze({allowed:false,limit_reached:true}),rate_limit_upsell:Object.freeze({banner_type:'usage_limit'})})};
    const originalBundle=${JSON.stringify(bundle)};
    const bannerBundle=${JSON.stringify(bannerBundle)};
    const bundle=fixtureCase==='missing_selector'?originalBundle.replace('n.rate_limit?.allowed!==!1','n.rate_limit?.allowed!==false'):fixtureCase==='cancel_startup'?originalBundle.replaceAll(${JSON.stringify(bundleUrl)},'app://-/assets/irrelevant.js'):originalBundle;
    globalThis.fetch=async url=>{
      if(url!==${JSON.stringify(bannerUrl)})throw Error('unexpected local fixture fetch: '+url);
      return new Response(bannerBundle,{status:200});
    };
    (0,eval)(bundle);if(fixtureCase!=='lazy_banner')(0,eval)(bannerBundle);
    globalThis.fixtureWarm={blocked:fixtureGate(),bannerVisible:globalThis.fixtureBanner?.()?.isUsageBannerEnabled??null};
    globalThis.fixtureReloadCount=0;
    globalThis.fixtureReload=()=>{
      fixtureReloadCount++;
      delete globalThis.__codexSwitchApiQuotaState;
      delete globalThis.fixtureBanner;
      (0,eval)(bundle);
      globalThis.fixtureColdBlocked=fixtureGate();
      const loadBanner=()=>{(0,eval)(bannerBundle);globalThis.fixtureFirstBanner=fixtureBanner().isUsageBannerEnabled};
      if(fixtureCase==='lazy_banner')setTimeout(loadBanner,500);else loadBanner();
      return true;
    };
    const server=http.createServer((req,res)=>{
      try {
        const url=new URL(req.url,'http://127.0.0.1');
        if(url.pathname!=='/probe'){res.writeHead(404);res.end('fixture route missing');return;}
        const mode=url.searchParams.get('mode');
        fixtureSetConfig(mode==='subscription'?{model_provider:'openai'}:{model_provider:'api',model_providers:{api:{requires_openai_auth:true,...(mode==='mixed'?{experimental_bearer_token:'fixture-bearer'}:{})}}});
        if(url.searchParams.get('reload')==='true')fixtureReload();
        const blocked=fixtureGate();
        const banner=globalThis.fixtureBanner?.();
        res.setHeader('content-type','application/json');
        res.end(JSON.stringify({blocked,bannerVisible:banner?.isUsageBannerEnabled??null,mode,warm:fixtureWarm,coldBlocked:globalThis.fixtureColdBlocked??null,firstBanner:globalThis.fixtureFirstBanner??null,reloads:fixtureReloadCount,sharedAllowed:fixtureState.quota.rate_limit.allowed,sharedBanner:fixtureState.quota.rate_limit_upsell.banner_type,unlocks:globalThis.__codexSwitchApiQuotaState?.unlockCount??0,error:globalThis.__codexSwitchApiQuotaState?.lastError??null}));
      } catch(error) {res.writeHead(500,{'content-type':'application/json'});res.end(JSON.stringify({error:String(error.stack??error)}));}
    });
    server.listen(0,'127.0.0.1',()=>console.log(JSON.stringify({ws:inspector.url(),control:'http://127.0.0.1:'+server.address().port})));
  `;
  const fixture = spawn(process.execPath, ['--inspect=127.0.0.1:0', '--input-type=module', '-e', fixtureCode], {
    windowsHide: true,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const fixtureExit = exitResult(fixture);
  let fixtureStderr = '';
  fixture.stderr.on('data', data => { fixtureStderr += data; });
  t.after(async () => { fixture.kill(); await fixtureExit; });
  const ready = await new Promise((resolve, reject) => {
    let stdout = '';
    fixture.once('error', reject);
    fixture.stdout.on('data', data => {
      stdout += data;
      const end = stdout.indexOf('\n');
      if (end >= 0) {
        try { resolve(JSON.parse(stdout.slice(0, end))); } catch (error) { reject(error); }
      }
    });
    fixtureExit.then(result => reject(new Error(`fixture exited: ${JSON.stringify(result)} ${fixtureStderr}`)), reject);
  });
  const cargo = spawn('cargo', ['test', 'isolated_api_quota_supervisor', '--', '--ignored', '--nocapture'], {
    cwd: path.join(root, 'src-tauri'),
    windowsHide: true,
    env: {
      ...process.env,
      CODEX_SWITCH_API_QUOTA_TEST_WS: ready.ws,
      CODEX_SWITCH_API_QUOTA_TEST_CONTROL: ready.control,
      CODEX_SWITCH_API_QUOTA_TEST_CASE: fixtureCase,
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  t.after(() => { if (cargo.exitCode === null) cargo.kill(); });
  let output = '';
  cargo.stdout.on('data', data => { output += data; });
  cargo.stderr.on('data', data => { output += data; });
  const result = await exitResult(cargo);
  t.diagnostic(output);
  assert.equal(result.code, 0, output);
}

for (const fixtureCase of ['warm_cache', 'lazy_banner', 'missing_selector', 'cancel_startup']) {
  test(`Rust/V8 remote quota fixture: ${fixtureCase}`, {skip:process.env.CODEX_SWITCH_TEST_CDP!=='1',timeout:240_000}, async t => runRustFixture(t,fixtureCase));
}
