import assert from 'node:assert/strict';
import test from 'node:test';
import { plan, selectCrates, workspaceCrates } from './affected-crates.mjs';

const ROOT = '/repo';

function member(name, directory, { dependencies = [], ci } = {}) {
  return {
    id: `${name} 0.1.0`,
    name,
    manifest_path: `${ROOT}/rust/crates/${directory}/Cargo.toml`,
    dependencies: dependencies.map((dependency) =>
      dependency.startsWith('registry:')
        ? { name: dependency.slice('registry:'.length) }
        : { name: dependency, path: `${ROOT}/rust/crates/${dependency.replace('lithe-', '')}` },
    ),
    metadata: ci ? { 'lithe-ci': ci } : null,
  };
}

function metadata(...packages) {
  return { packages, workspace_members: packages.map((pkg) => pkg.id) };
}

// platform <- git-host <- core, platform <- platform-tests (dev), lone.
const crates = workspaceCrates(
  metadata(
    member('lithe-platform', 'platform', { ci: { hosts: ['macos', 'windows'] } }),
    member('lithe-platform-tests', 'platform-tests', {
      dependencies: ['lithe-platform', 'lithe-git-host'],
      ci: { hosts: ['macos', 'windows'] },
    }),
    member('lithe-git-host', 'git-host', { dependencies: ['lithe-platform', 'registry:libc'] }),
    member('lithe-core', 'core', { dependencies: ['lithe-git-host'] }),
    member('lithe-remote', 'remote', { ci: { hosts: ['macos'], 'test-features': ['server'] } }),
    member('lithe-lone', 'lone'),
  ),
  ROOT,
);

const names = (set) => [...set].sort();

test('workspace crates keep only dependencies on other members', () => {
  const gitHost = crates.find((crate) => crate.name === 'lithe-git-host');
  assert.equal(gitHost.directory, 'rust/crates/git-host');
  assert.deepEqual(gitHost.dependencies, ['lithe-platform']);
  assert.deepEqual(gitHost.hosts, []);
});

test('an unknown host is rejected', () => {
  assert.throws(
    () => workspaceCrates(metadata(member('lithe-x', 'x', { ci: { hosts: ['linux'] } })), ROOT),
    /unknown lithe-ci host "linux"/,
  );
});

test('a changed crate selects itself and every crate depending on it', () => {
  assert.deepEqual(names(selectCrates(crates, ['rust/crates/git-host/src/lib.rs'])), [
    'lithe-core',
    'lithe-git-host',
    'lithe-platform-tests',
  ]);
  assert.deepEqual(names(selectCrates(crates, ['rust/crates/lone/README.md'])), ['lithe-lone']);
});

test('a crate directory is not mistaken for another with the same prefix', () => {
  assert.deepEqual(names(selectCrates(crates, ['rust/crates/platform-tests/src/lib.rs'])), [
    'lithe-platform-tests',
  ]);
});

test('workspace files, contract fixtures and the checks select every crate', () => {
  for (const file of [
    'rust/Cargo.lock',
    'rust/fixtures/git/case.json',
    'rust/crates/removed/src/lib.rs',
    'shared/plugins/contract/v1/README.md',
    'scripts/rust-core/verify-rust-core-comments.sh',
    '.github/workflows/rust-core.yml',
  ]) {
    assert.equal(selectCrates(crates, [file]).size, crates.length, file);
  }
  assert.equal(selectCrates(crates, null).size, crates.length);
});

test('files no crate builds select nothing, and e2e tests select lithe-core', () => {
  assert.equal(selectCrates(crates, ['README.md', 'shared/plugins/sdk/node/index.mjs']).size, 0);
  assert.deepEqual(names(selectCrates(crates, ['shared/plugins/sdk/e2e/plugin_sdk/mod.rs'])), [
    'lithe-core',
  ]);
});

test('the plan tests declared crates on their hosts with their features', () => {
  const result = plan(crates, selectCrates(crates, ['rust/crates/platform/src/lib.rs', 'rust/crates/remote/src/lib.rs']));
  assert.equal(result.rust, true);
  assert.equal(
    result.packages,
    '-p lithe-platform -p lithe-platform-tests -p lithe-git-host -p lithe-core -p lithe-remote',
  );
  assert.equal(
    result.testArgs,
    '-p lithe-platform -p lithe-platform-tests -p lithe-git-host -p lithe-core -p lithe-remote --features lithe-remote/server',
  );
  assert.equal(result.crateDirectories, 'platform platform-tests git-host core remote');
  assert.deepEqual(result.hosts, [
    {
      os: 'macos',
      name: 'macOS',
      runner: 'macos-latest',
      'test-args': '-p lithe-platform -p lithe-platform-tests -p lithe-remote --features lithe-remote/server',
    },
    {
      os: 'windows',
      name: 'Windows',
      runner: 'windows-latest',
      'test-args': '-p lithe-platform -p lithe-platform-tests',
    },
  ]);
});

test('the plan names the workspace when every crate is selected', () => {
  const result = plan(crates, selectCrates(crates, null));
  assert.equal(result.packages, '--workspace');
  assert.equal(result.testArgs, '--workspace --features lithe-remote/server');
  assert.equal(result.crateDirectories, '');
});

test('the plan builds nothing when no crate is selected', () => {
  const result = plan(crates, new Set());
  assert.deepEqual(result, { rust: false, packages: '', testArgs: '', crateDirectories: '', hosts: [] });
});
