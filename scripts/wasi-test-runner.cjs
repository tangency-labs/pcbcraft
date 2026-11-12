// Run Rust's WASI test harness with Node 22 or later; no filesystem preopens are needed.
// Cargo supplies the module path followed by the ordinary libtest filter/options.
const fs = require('node:fs');
const { WASI } = require('node:wasi');
const wasi = new WASI({ version: 'preview1', args: process.argv.slice(2), env: {}, preopens: {} });
const moduleBytes = fs.readFileSync(process.argv[2]);
WebAssembly.instantiate(moduleBytes, wasi.getImportObject()).then(({instance}) => {
  wasi.start(instance);
}).catch(error => { console.error(error); process.exitCode = 1; });
