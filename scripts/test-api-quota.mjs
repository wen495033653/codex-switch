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
  const $={},_O={},_b={},cv={},yJe={};
  const Ud=(_,read)=>({read:()=>read({get:e})});
  function e(atom) { if(atom===_O)return {authMethod:"chatgpt"}; if(atom===_b)return {data:globalThis.fixtureState.quota}; if(atom===cv)return {data:{model_provider:globalThis.fixtureState.provider}}; throw Error("unexpected fixture atom"); }
  let Yz;
  Yz=Ud($,({get:e})=>{let t=e(_O),n=e(_b).data;if(t.authMethod!==\`chatgpt\`||n.rate_limit?.allowed!==!1)return!1;return!0});
  function providerCheck(e,t){let r=null,i="local";let{data:a}=e.get(cv,{cwd:r,hostId:i});return a==null||(e.get(yJe,t)??a.model_provider??\`openai\`)!==\`openai\`?null:true}
  globalThis.fixtureGate=()=>Yz.read();
})();
//# sourceURL=${bundleUrl}`;

test('quota locator uniquely identifies the current selector and effective config descriptor', () => {
  const location = locate(bundle, bundleUrl);
  assert.equal(location.atom, 'Yz');
  assert.equal(location.getter, 'e');
  assert.equal(location.data, 'n');
  assert.equal(location.config, 'cv');
  assert.equal(bundle.split('\n')[location.lineNumber].slice(location.columnNumber, location.columnNumber + 3), 'if(');
  assert.match(location.condition, /provider === "api"/);
  assert.match(location.condition, /n = \{ \.\.\.n, rate_limit:/);
  assert.throws(() => locate('', bundleUrl), /selector count=0/);
  assert.throws(() => locate(bundle + bundle, bundleUrl), /selector count=2/);
  assert.throws(() => locate(bundle.replaceAll('rate_limit?.allowed', 'quota.allowed'), bundleUrl), /guard missing/);
  assert.throws(() => locate(bundle.replaceAll('model_provider', 'unknown_provider'), bundleUrl), /descriptor count=0/);
});

test('condition clones only local quota data and respects provider on every evaluation', () => {
  const condition = locate(bundle, bundleUrl).condition;
  const shared = Object.freeze({ rate_limit: Object.freeze({ allowed: false, limit_reached: true }) });
  const context = vm.createContext({ provider: 'api', shared });
  vm.runInContext(`const cv={};const e=()=>({data:{model_provider:provider}});let n=shared;`, context);
  for (const [provider, expected] of [['api', true], ['openai', false], ['api', true]]) {
    context.provider = provider;
    vm.runInContext('n=shared;', context);
    assert.equal(vm.runInContext(condition, context), false);
    assert.equal(vm.runInContext('n.rate_limit.allowed', context), expected);
    assert.equal(shared.rate_limit.allowed, false);
    assert.equal(vm.runInContext('n.rate_limit.limit_reached', context), true);
  }
  vm.runInContext('n=shared;', context);
  context.provider = undefined;
  vm.runInContext(condition, context);
  assert.equal(vm.runInContext('n.rate_limit.allowed', context), false);
});

function exitResult(child) {
  return new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolve({ code, signal }));
  });
}

test('Rust supervisor keeps real V8 breakpoints through repeated calls, provider switches, reload and stop', {
  skip: process.env.CODEX_SWITCH_TEST_CDP !== '1',
  timeout: 240_000,
}, async t => {
  const fixtureCode = `
    import http from 'node:http';
    import inspector from 'node:inspector';
    globalThis.fixtureState={provider:'openai',quota:Object.freeze({rate_limit:Object.freeze({allowed:false,limit_reached:true})})};
    const bundle=${JSON.stringify(bundle)};
    (0,eval)(bundle);
    const server=http.createServer((req,res)=>{
      try {
        const url=new URL(req.url,'http://127.0.0.1');
        if(url.pathname!=='/probe'){res.writeHead(404);res.end('fixture route missing');return;}
        fixtureState.provider=url.searchParams.get('provider');
        if(url.searchParams.get('reload')==='true'){delete globalThis.__codexSwitchApiQuotaState;(0,eval)(bundle);}
        const blocked=fixtureGate();
        res.setHeader('content-type','application/json');
        res.end(JSON.stringify({blocked,provider:fixtureState.provider,sharedAllowed:fixtureState.quota.rate_limit.allowed,unlocks:globalThis.__codexSwitchApiQuotaState?.unlockCount??0,error:globalThis.__codexSwitchApiQuotaState?.lastError??null}));
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
  assert.match(output, /API quota after stop:/);
});
