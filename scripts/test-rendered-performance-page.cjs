// Exercise the real rendered page loader, result handling and renderer cleanup.
const {readFileSync} = require('node:fs');
const {join} = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const assert = require('node:assert/strict');
const source = readFileSync(join(__dirname, '../tools/performance/rendered.html'), 'utf8').match(/<script type="module">([\s\S]*?)<\/script>/)[1];

async function page({httpStatus = 200, failure = false, stalled = false, late = false} = {}) {
  const output = {textContent: '', hidden: false};
  let destroyed = 0;
  let finishStartup;
  const context = vm.createContext({URLSearchParams, AbortSignal, location: {search: '?artifact=a&environment=e&iterations=1'},
    document: {querySelector: () => output},
    setTimeout: (callback, delay) => setTimeout(callback, delay === 30000 ? 30 : 0), clearTimeout,
    fetch: async () => ({ok: httpStatus === 200, status: httpStatus, text: async () => '{}'})});
  const dependency = new vm.SyntheticModule(['default', 'start_rendered'], function () {
    this.setExport('default', async () => {});
    const runner = {poll: () => {
      if (failure) throw Error('GPU readback failed');
      return JSON.stringify({captures: 1});
    }, destroy: () => {destroyed++;}};
    this.setExport('start_rendered', () => stalled ? new Promise(resolve => {finishStartup = () => resolve(runner);}) : Promise.resolve(runner));
  }, {context});
  await dependency.link(() => {throw Error('unexpected import');});
  await dependency.evaluate();
  const module = new vm.SourceTextModule(source, {context, importModuleDynamically: async () => dependency});
  await module.link(() => {throw Error('unexpected static import');});
  await module.evaluate();
  if (late) {finishStartup(); await new Promise(resolve => setImmediate(resolve));}
  return {context, output, destroyed};
}

test('rendered results are retained and the runner is released', async () => {
  const {context, output, destroyed} = await page();
  assert.equal(context.performanceReport.captures, 1);
  assert.equal(output.hidden, false);
  assert.equal(destroyed, 1);
});

test('rendering failure stays an error and still destroys the runner', async () => {
  const {context, output, destroyed} = await page({failure: true});
  assert.match(context.performanceError, /GPU readback failed/);
  assert.equal(context.performanceReport, undefined);
  assert.equal(output.hidden, false);
  assert.equal(destroyed, 1);
});

test('failed fixture reads cannot start rendering', async () => {
  const {context, destroyed} = await page({httpStatus: 404});
  assert.match(context.performanceError, /HTTP 404/);
  assert.equal(destroyed, 0);
});

test('stalled renderer startup fails and releases a late runner', async () => {
  const {context, output, destroyed} = await page({stalled: true, late: true});
  assert.match(context.performanceError, /Renderer startup timed out/);
  assert.equal(context.performanceReport, undefined);
  assert.equal(output.hidden, false);
  assert.equal(destroyed, 1);
});
