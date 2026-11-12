// Compile original WASI programs and exercise the production Cargo runner as a subprocess.
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

test('the WASI runner preserves successful and failing program exit codes', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'pcbcraft-wasi-runner-'));
  try {
    for (const exitCode of [0, 23]) {
      const source = path.join(directory, 'exit.rs');
      const binary = path.join(directory, 'exit.wasm');
      fs.writeFileSync(source, `fn main() { std::process::exit(${exitCode}); }\n`);
      const compiled = spawnSync('rustc', ['--target', 'wasm32-wasip1', source, '-o', binary], { encoding: 'utf8' });
      assert.ifError(compiled.error);
      assert.equal(compiled.status, 0, compiled.stderr);
      const executed = spawnSync(process.execPath, ['--no-warnings', path.join(__dirname, 'wasi-test-runner.cjs'), binary], { encoding: 'utf8' });
      assert.ifError(executed.error);
      assert.equal(executed.status, exitCode, executed.stderr);
    }
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
