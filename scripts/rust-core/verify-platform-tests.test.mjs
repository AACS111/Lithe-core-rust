import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {
  HOST_CONFIGURATIONS,
  crateTests,
  evaluate,
  parsePredicate,
  runningHosts,
  scanSource,
  tokenize,
  verify,
} from './verify-platform-tests.mjs';

const predicate = (source) => parsePredicate(tokenize(source));
const on = (source) =>
  Object.fromEntries(Object.entries(HOST_CONFIGURATIONS).map(([host, cfg]) => [host, evaluate(predicate(source), cfg)]));
const scan = (source) => scanSource(source, { file: 'src/lib.rs', directory: '/crate/src' });
const where = (source) => scan(source).tests.map((found) => [found.name, runningHosts(found, ['linux', 'macos', 'windows'])]);

test('comments, strings and lifetimes hide nothing and invent nothing', () => {
  const source = String.raw`
    // #[test] fn commented() {}
    /* nested /* #[test] */ fn commented_too() {} */
    const BRACES: &str = "}{ #[test]";
    const RAW: &str = r#"" } #[test] "#;
    fn borrow<'a>(text: &'a str) -> char { let _ = b'}'; '{' }
    fn r#match() {}
    #[test]
    fn real() {}
  `;
  assert.deepEqual(where(source), [['real', ['linux', 'macos', 'windows']]]);
  const kinds = tokenize("'a 'b' '\\n' b'x'").map((token) => token.kind);
  assert.deepEqual(kinds, ['lifetime', 'literal', 'literal', 'literal']);
});

test('predicates are true, false or unknown on each host', () => {
  assert.deepEqual(on('unix'), { linux: true, macos: true, windows: false });
  assert.deepEqual(on('target_os = "macos"'), { linux: false, macos: true, windows: false });
  assert.deepEqual(on('not(windows)'), { linux: true, macos: true, windows: false });
  assert.deepEqual(on('all(test, target_family = "windows")'), { linux: false, macos: false, windows: true });
  assert.deepEqual(on('feature = "server"'), { linux: null, macos: null, windows: null });
  assert.deepEqual(on('any(windows, feature = "server")'), { linux: null, macos: null, windows: true });
  assert.deepEqual(on('all(unix, feature = "server")'), { linux: null, macos: null, windows: false });
});

test('cfg on tests, modules and files decides where a test runs', () => {
  const source = `
    #![cfg(unix)]
    #[test] fn on_unix() {}
    #[cfg(windows)]
    pub(crate) mod windows {
        #[tokio::test(flavor = "current_thread")]
        async fn never() {}
    }
    mod plain {
        #[cfg(target_os = "macos")] #[test] fn on_macos() {}
        #[cfg(feature = "server")] #[test] fn behind_a_feature() {}
    }
  `;
  assert.deepEqual(where(source), [
    ['on_unix', ['linux', 'macos']],
    ['never', []],
    ['on_macos', ['macos']],
    ['behind_a_feature', ['linux', 'macos']],
  ]);
});

test('cfg_attr can make a function a test or ignore it on some hosts', () => {
  const source = `
    #[cfg_attr(windows, test)] fn tested_on_windows() {}
    #[test] #[cfg_attr(not(windows), ignore = "needs a job object")] fn ignored_elsewhere() {}
    #[test] #[cfg_attr(feature = "slow", ignore)] fn maybe_ignored() {}
    #[cfg_attr(unix, allow(dead_code))] fn helper() {}
  `;
  assert.deepEqual(where(source), [
    ['tested_on_windows', ['windows']],
    ['ignored_elsewhere', ['windows']],
    ['maybe_ignored', ['linux', 'macos', 'windows']],
  ]);
});

test('module declarations name the files a crate compiles', () => {
  const { modules } = scan(`
    #[cfg(windows)] mod windows;
    #[path = "support/shared.rs"] mod shared;
    mod outer { mod inner; }
  `);
  assert.deepEqual(
    modules.map(({ name, candidates }) => [name, candidates]),
    [
      ['windows', ['/crate/src/windows.rs', '/crate/src/windows/mod.rs']],
      ['shared', ['src/support/shared.rs']],
      ['inner', ['/crate/src/outer/inner.rs', '/crate/src/outer/inner/mod.rs']],
    ],
  );
  assert.equal(evaluate(modules[0].conditions[0], HOST_CONFIGURATIONS.linux), false);
});

/** A crate in a new temporary directory, removed after the test. */
function temporaryCrate(t, files) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'lithe-platform-tests-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const [file, source] of Object.entries(files)) {
    mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
    writeFileSync(path.join(root, file), source);
  }
  return root;
}

test('a crate is scanned through every module file its targets reach', (t) => {
  const root = temporaryCrate(t, {
    'crate/src/lib.rs': '#[cfg(windows)] mod windows;\nmod host;\n#[cfg(test)] mod tests;\nmod missing;\n',
    'crate/src/windows.rs': 'mod job;\n',
    'crate/src/windows/job.rs': '#[test] fn job_object() {}\n',
    'crate/src/host.rs': '#[cfg(unix)]\nmod unix { #[test] fn unix_host() {} }\n',
    'crate/src/tests/mod.rs': 'mod cases;\n',
    'crate/src/tests/cases.rs': '#[test] fn anywhere() {}\n',
    'crate/tests/api.rs': '#[test] fn integration() {}\n',
  });
  const { tests, problems } = crateTests(
    { name: 'lithe-crate', hosts: [], roots: ['crate/src/lib.rs', 'crate/tests/api.rs'] },
    root,
  );
  assert.deepEqual(
    tests.map((found) => [found.file, found.name, runningHosts(found, ['linux', 'windows'])]),
    [
      ['crate/src/windows/job.rs', 'job_object', ['windows']],
      ['crate/src/host.rs', 'unix_host', ['linux']],
      ['crate/src/tests/cases.rs', 'anywhere', ['linux', 'windows']],
      ['crate/tests/api.rs', 'integration', ['linux', 'windows']],
    ],
  );
  assert.deepEqual(problems, ['crate/src/lib.rs:4: cannot find the file of module `missing`']);
});

test('a test must run on a host CI tests its crate on', (t) => {
  const root = temporaryCrate(t, {
    'linux-only/src/lib.rs': '#[cfg(windows)] #[test] fn on_windows() {}\n#[cfg(unix)] #[test] fn on_unix() {}\n',
    'declared/src/lib.rs': '#[cfg(windows)] #[test] fn on_windows() {}\n',
    'ignored/src/lib.rs': '#[test] #[cfg_attr(unix, ignore)] fn not_on_unix() {}\n',
  });
  const crate = (name, hosts = []) => ({ name, hosts, roots: [`${name}/src/lib.rs`] });
  const { problems, gated } = verify([crate('linux-only'), crate('declared', ['windows']), crate('ignored')], root);
  assert.deepEqual(problems, [
    'linux-only: linux-only/src/lib.rs:1 `on_windows` runs on none of the hosts CI tests this crate on (linux)',
    'ignored: ignored/src/lib.rs:1 `not_on_unix` runs on none of the hosts CI tests this crate on (linux)',
  ]);
  assert.equal(gated.length, 4);
});
