// Exercise the diagnostic page's actual module with failed imports and HTTP requests.
// Run with --experimental-vm-modules; the production WASM replay has browser smoke coverage.
const {readFileSync} = require('node:fs');
const {join} = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const assert = require('node:assert/strict');
const html = readFileSync(join(__dirname, '../tools/performance/index.html'), 'utf8');
const source = html.match(/<script type="module">([\s\S]*?)<\/script>/)[1];

/** Run the actual page module with an isolated DOM and controlled module/HTTP outcomes. */
async function page({missingModule = false, httpStatus = 200, iterations = '1'} = {}) {
  const output = {textContent: 'Running CPU diagnostics…'};
  const context = vm.createContext({
    URLSearchParams,
    location: {search: `?artifact=fixture.json&environment=environment.json&iterations=${iterations}`},
    document: {querySelector: () => output},
    fetch: async () => ({ok: httpStatus === 200, status: httpStatus, text: async () => '{}'}),
  });
  const dependency = new vm.SyntheticModule(['default', 'run_performance'], function () {
    this.setExport('default', async () => {});
    this.setExport('run_performance', () => JSON.stringify({iterations: 1}));
  }, {context});
  await dependency.link(() => {throw new Error('unexpected nested import');});
  await dependency.evaluate();
  const module = new vm.SourceTextModule(source, {context, importModuleDynamically: async () => {
    if (missingModule) throw new Error('missing generated module');
    return dependency;
  }});
  // A static import failure occurs here, outside the page's error handler, and fails this test.
  await module.link(() => {throw new Error('imports must execute inside the error handler');});
  await module.evaluate();
  return {context, output};
}

test('a missing generated module becomes a visible diagnostic failure', async () => {
  const {context, output} = await page({missingModule: true});
  assert.match(context.performanceError, /missing generated module/);
  assert.equal(output.textContent, context.performanceError);
  assert.equal(context.performanceReport, undefined);
});

test('HTTP and iteration failures do not masquerade as a report', async () => {
  for (const [options, expected] of [[{httpStatus: 404}, /HTTP 404/], [{iterations: '0'}, /Invalid iterations/]]) {
    const {context, output} = await page(options);
    assert.match(context.performanceError, expected);
    assert.equal(output.textContent, context.performanceError);
    assert.equal(context.performanceReport, undefined);
  }
});

test('successful results remain available as JSON and visible text', async () => {
  const {context, output} = await page();
  assert.equal(context.performanceError, undefined);
  assert.equal(context.performanceReport.iterations, 1);
  assert.deepEqual(JSON.parse(output.textContent), {iterations: 1});
});
