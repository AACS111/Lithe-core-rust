#!/usr/bin/env node
// Chooses what Rust Core CI builds and tests for a change: the crates whose
// files changed, every crate that depends on them, and, among those, the
// crates whose tests must also run on a real macOS or Windows host.
//
// Linux runs every selected crate. A crate whose behaviour differs per
// operating system declares the other hosts in its Cargo.toml:
//
//   [package.metadata.lithe-ci]
//   hosts = ["macos", "windows"]
//   test-features = ["server"]   # features its tests need, on every host
//
// Usage:
//   node scripts/rust-core/affected-crates.mjs [--base <rev>] [--all] [--github]
//
// Without --all, the changed files are those since the merge base of <rev>
// (default origin/main) and HEAD, including uncommitted and untracked files.
// --github also appends the selection to $GITHUB_OUTPUT.

import { execFileSync } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

/** Hosts that run declared crates' tests besides Linux, with their GitHub runners. */
export const HOSTS = {
  macos: { name: 'macOS', runner: 'macos-latest' },
  windows: { name: 'Windows', runner: 'windows-latest' },
};

/**
 * Paths outside crate directories that can change the build or tests of any
 * crate. `rust/` holds the workspace manifest, the lock file, and the
 * fixtures, contracts and log catalog that several crates read.
 */
export const EVERY_CRATE = [
  'rust/',
  'shared/plugins/contract/',
  'scripts/rust-core/',
  'scripts/logging/',
  '.github/workflows/rust-core.yml',
];

/** Paths outside crate directories that only one crate builds. */
export const READ_BY = [
  // The plugin-sdk-e2e feature of lithe-core compiles these tests, and the
  // lints enable every feature.
  { prefix: 'shared/plugins/sdk/e2e/', crate: 'lithe-core' },
];

/** The workspace members of `cargo metadata --no-deps` output. */
export function workspaceCrates(metadata, root = ROOT) {
  const members = metadata.packages.filter((pkg) => metadata.workspace_members.includes(pkg.id));
  const nameByDirectory = new Map(members.map((pkg) => [path.dirname(pkg.manifest_path), pkg.name]));
  return members.map((pkg) => {
    const ci = pkg.metadata?.['lithe-ci'] ?? {};
    const hosts = ci.hosts ?? [];
    for (const host of hosts) {
      if (!Object.hasOwn(HOSTS, host)) {
        throw new Error(`${pkg.name}: unknown lithe-ci host "${host}"`);
      }
    }
    return {
      name: pkg.name,
      directory: path.relative(root, path.dirname(pkg.manifest_path)).split(path.sep).join('/'),
      // Every dependency kind counts: a dev-dependency change can break tests.
      dependencies: [
        ...new Set(
          pkg.dependencies.flatMap((dependency) =>
            nameByDirectory.has(dependency.path) ? [nameByDirectory.get(dependency.path)] : [],
          ),
        ),
      ],
      hosts,
      testFeatures: ci['test-features'] ?? [],
      // Source files that start a compiled target; benches and examples
      // never run in a test build.
      roots: (pkg.targets ?? [])
        .filter((target) => !target.kind.some((kind) => ['bench', 'example', 'custom-build'].includes(kind)))
        .map((target) => path.relative(root, target.src_path).split(path.sep).join('/')),
    };
  });
}

/**
 * Names of the crates that a change to `files`, repository-relative paths
 * with forward slashes, can affect. `null` stands for every file.
 */
export function selectCrates(crates, files) {
  const every = new Set(crates.map((crate) => crate.name));
  if (files === null) {
    return every;
  }
  const selected = new Set();
  for (const file of files) {
    const owner = crates.find((crate) => file.startsWith(`${crate.directory}/`));
    if (owner) {
      selected.add(owner.name);
    } else if (EVERY_CRATE.some((prefix) => file.startsWith(prefix))) {
      return every;
    } else {
      for (const { prefix, crate } of READ_BY) {
        if (file.startsWith(prefix) && every.has(crate)) {
          selected.add(crate);
        }
      }
    }
  }
  const dependents = new Map();
  for (const crate of crates) {
    for (const dependency of crate.dependencies) {
      dependents.set(dependency, [...(dependents.get(dependency) ?? []), crate.name]);
    }
  }
  const pending = [...selected];
  while (pending.length > 0) {
    for (const dependent of dependents.get(pending.pop()) ?? []) {
      if (!selected.has(dependent)) {
        selected.add(dependent);
        pending.push(dependent);
      }
    }
  }
  return selected;
}

/** Cargo arguments that test `crates`, or the whole workspace. */
function testArguments(crates, wholeWorkspace) {
  const packages = wholeWorkspace ? ['--workspace'] : crates.flatMap((crate) => ['-p', crate.name]);
  const features = crates.flatMap((crate) =>
    crate.testFeatures.map((feature) => `${crate.name}/${feature}`),
  );
  return [...packages, ...(features.length > 0 ? ['--features', features.join(',')] : [])].join(' ');
}

/** What each CI job runs for the crates in `selected`. */
export function plan(crates, selected) {
  const chosen = crates.filter((crate) => selected.has(crate.name));
  const whole = chosen.length === crates.length;
  return {
    rust: chosen.length > 0,
    packages: whole ? '--workspace' : chosen.flatMap((crate) => ['-p', crate.name]).join(' '),
    testArgs: chosen.length > 0 ? testArguments(chosen, whole) : '',
    // The comment check covers every crate when it receives none.
    crateDirectories: whole ? '' : chosen.map((crate) => path.posix.basename(crate.directory)).join(' '),
    hosts: Object.entries(HOSTS).flatMap(([os, host]) => {
      const declared = chosen.filter((crate) => crate.hosts.includes(os));
      return declared.length > 0
        ? [{ os, name: host.name, runner: host.runner, 'test-args': testArguments(declared, false) }]
        : [];
    }),
  };
}

function git(...args) {
  return execFileSync('git', args, { cwd: ROOT, encoding: 'utf8' });
}

/** Files changed since the merge base of `base` and HEAD, also those not yet committed. */
export function changedFiles(base) {
  const mergeBase = git('merge-base', base, 'HEAD').trim();
  // Without rename detection a moved file also reports the path it left.
  const tracked = git('diff', '--name-only', '--no-renames', mergeBase).split('\n');
  const untracked = git('ls-files', '--others', '--exclude-standard').split('\n');
  return [...new Set([...tracked, ...untracked].filter(Boolean))];
}

/** `cargo metadata --no-deps` output for the Rust Core workspace. */
export function readMetadata() {
  const output = execFileSync(
    'cargo',
    ['metadata', '--no-deps', '--format-version', '1', '--manifest-path', path.join(ROOT, 'rust', 'Cargo.toml')],
    { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  );
  return JSON.parse(output);
}

function parseArguments(args) {
  const options = { base: 'origin/main', all: false, github: false };
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === '--all') {
      options.all = true;
    } else if (argument === '--github') {
      options.github = true;
    } else if (argument === '--base' && index + 1 < args.length) {
      index += 1;
      options.base = args[index];
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  return options;
}

function main() {
  const options = parseArguments(process.argv.slice(2));
  const crates = workspaceCrates(readMetadata());
  const files = options.all ? null : changedFiles(options.base);
  const result = plan(crates, selectCrates(crates, files));
  console.log(`changed files: ${files === null ? 'all' : files.length}`);
  console.log(`linux: ${result.rust ? result.testArgs : 'nothing to build'}`);
  for (const host of result.hosts) {
    console.log(`${host.os}: ${host['test-args']}`);
  }
  if (options.github) {
    const output = process.env.GITHUB_OUTPUT;
    if (!output) {
      throw new Error('GITHUB_OUTPUT is not set');
    }
    appendFileSync(
      output,
      [
        `rust=${result.rust}`,
        `packages=${result.packages}`,
        `test-args=${result.testArgs}`,
        `crate-directories=${result.crateDirectories}`,
        `hosts=${JSON.stringify(result.hosts)}`,
        '',
      ].join('\n'),
    );
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main();
}
