#!/usr/bin/env node
// Fails when a Rust Core test runs on none of the hosts CI runs its crate on.
//
// CI tests every crate on Linux and a crate on macOS or Windows only when it
// declares that host under [package.metadata.lithe-ci] (see
// affected-crates.mjs). A test that `cfg` compiles out, or `cfg_attr` ignores,
// on each of those hosts would never run. Tests that need a real macOS or
// Windows host belong in lithe-platform-tests, which CI runs on every host;
// CONTRIBUTING.md explains where each kind of test goes.
//
// Usage:
//   node scripts/rust-core/verify-platform-tests.mjs [--list]
//
// --list also prints every test that some host compiles out or ignores.

import { readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { ROOT, readMetadata, workspaceCrates } from './affected-crates.mjs';

/**
 * The configuration each CI host compiles tests with. Options it does not
 * list, such as features, are unknown: a test behind them may run.
 */
export const HOST_CONFIGURATIONS = {
  linux: {
    flags: { unix: true, windows: false, test: true },
    values: { target_os: 'linux', target_family: 'unix', target_vendor: 'unknown', target_env: 'gnu' },
  },
  macos: {
    flags: { unix: true, windows: false, test: true },
    values: { target_os: 'macos', target_family: 'unix', target_vendor: 'apple', target_env: '' },
  },
  windows: {
    flags: { unix: false, windows: true, test: true },
    values: { target_os: 'windows', target_family: 'windows', target_vendor: 'pc', target_env: 'msvc' },
  },
};

const IDENTIFIER = /[A-Za-z0-9_\u0080-\uffff]/;
const RAW_STRING = /^[bc]?r(#*)"/;

/**
 * Splits Rust source into identifiers, literals, lifetimes and punctuation,
 * dropping whitespace and comments. `::` is one token; every other
 * punctuation character is its own.
 */
export function tokenize(source) {
  const tokens = [];
  let line = 1;
  let index = 0;
  const advance = (end) => {
    for (let position = index; position < end; position += 1) {
      if (source[position] === '\n') {
        line += 1;
      }
    }
    index = end;
  };
  const emit = (kind, end, value) => {
    tokens.push({ kind, text: source.slice(index, end), value, line });
    advance(end);
  };
  // The end of a quoted literal whose body starts at `start`.
  const quoted = (start, quote) => {
    let position = start;
    while (position < source.length && source[position] !== quote) {
      position += source[position] === '\\' ? 2 : 1;
    }
    return position + 1;
  };
  while (index < source.length) {
    const character = source[index];
    const next = source[index + 1];
    if (/\s/.test(character)) {
      advance(index + 1);
    } else if (character === '/' && next === '/') {
      const end = source.indexOf('\n', index);
      advance(end === -1 ? source.length : end);
    } else if (character === '/' && next === '*') {
      let depth = 1;
      let position = index + 2;
      while (position < source.length && depth > 0) {
        if (source.startsWith('/*', position)) {
          depth += 1;
          position += 2;
        } else if (source.startsWith('*/', position)) {
          depth -= 1;
          position += 2;
        } else {
          position += 1;
        }
      }
      advance(position);
    } else if (/[bcr]/.test(character) && RAW_STRING.test(source.slice(index, index + 260))) {
      // Raw string: r"…", r#"…"#, br"…", cr"…".
      const opening = RAW_STRING.exec(source.slice(index, index + 260));
      const body = index + opening[0].length;
      const closing = `"${opening[1]}`;
      const end = source.indexOf(closing, body);
      const stop = end === -1 ? source.length : end;
      emit('literal', stop + closing.length, source.slice(body, stop));
    } else if (character === '"' || (/[bc]/.test(character) && next === '"')) {
      const body = character === '"' ? index + 1 : index + 2;
      const end = quoted(body, '"');
      emit('literal', end, source.slice(body, end - 1));
    } else if (character === "'" || (character === 'b' && next === "'")) {
      const quote = character === 'b' ? index + 1 : index;
      if (source[quote + 1] === '\\') {
        emit('literal', quoted(quote + 1, "'"));
      } else {
        const width = source.codePointAt(quote + 1) > 0xffff ? 2 : 1;
        if (source[quote + 1 + width] === "'") {
          emit('literal', quote + 2 + width);
        } else {
          let end = quote + 1;
          while (end < source.length && IDENTIFIER.test(source[end])) {
            end += 1;
          }
          emit('lifetime', end);
        }
      }
    } else if (character === 'r' && next === '#' && IDENTIFIER.test(source[index + 2] ?? '')) {
      let end = index + 2;
      while (end < source.length && IDENTIFIER.test(source[end])) {
        end += 1;
      }
      tokens.push({ kind: 'identifier', text: source.slice(index + 2, end), line });
      advance(end);
    } else if (IDENTIFIER.test(character)) {
      const number = /[0-9]/.test(character);
      let end = index + 1;
      while (
        end < source.length &&
        (IDENTIFIER.test(source[end]) || (number && source[end] === '.' && /[0-9]/.test(source[end + 1] ?? '')))
      ) {
        end += 1;
      }
      emit(number ? 'literal' : 'identifier', end);
    } else if (character === ':' && next === ':') {
      emit('punctuation', index + 2);
    } else {
      emit('punctuation', index + 1);
    }
  }
  return tokens;
}

const CLOSERS = { '(': ')', '[': ']', '{': '}' };

/** The index after the group that opens at `open`. */
function skipGroup(tokens, open) {
  const stack = [];
  for (let index = open; index < tokens.length; index += 1) {
    const { kind, text } = tokens[index];
    if (kind !== 'punctuation') {
      continue;
    }
    if (CLOSERS[text]) {
      stack.push(CLOSERS[text]);
    } else if (text === stack[stack.length - 1]) {
      stack.pop();
      if (stack.length === 0) {
        return index + 1;
      }
    }
  }
  return tokens.length;
}

/** `tokens` split at commas outside any group, without empty parts. */
function splitArguments(tokens) {
  const parts = [[]];
  for (let index = 0; index < tokens.length; ) {
    const { kind, text } = tokens[index];
    if (kind === 'punctuation' && CLOSERS[text]) {
      const end = skipGroup(tokens, index);
      parts[parts.length - 1].push(...tokens.slice(index, end));
      index = end;
    } else {
      if (kind === 'punctuation' && text === ',') {
        parts.push([]);
      } else {
        parts[parts.length - 1].push(tokens[index]);
      }
      index += 1;
    }
  }
  return parts.filter((part) => part.length > 0);
}

/** A `cfg` predicate: `{ all | any: [...] }`, `{ not: predicate }` or an option. */
export function parsePredicate(tokens) {
  const [head, ...rest] = tokens;
  if (!head || head.kind !== 'identifier') {
    // A macro fragment such as `$condition`: its value is unknown.
    return { option: '' };
  }
  if (rest[0]?.text === '(') {
    const items = splitArguments(rest.slice(1, skipGroup(rest, 0) - 1)).map(parsePredicate);
    if (head.text === 'not') {
      return { not: items[0] };
    }
    return { [head.text === 'all' ? 'all' : 'any']: items };
  }
  if (rest[0]?.text === '=') {
    return { option: head.text, value: rest[1]?.value ?? rest[1]?.text };
  }
  return { option: head.text };
}

/** Whether `predicate` holds on `host`: true, false, or null when unknown. */
export function evaluate(predicate, host) {
  if (predicate.all || predicate.any) {
    const values = (predicate.all ?? predicate.any).map((item) => evaluate(item, host));
    const decisive = Boolean(predicate.any);
    if (values.includes(decisive)) {
      return decisive;
    }
    return values.includes(null) ? null : !decisive;
  }
  if (predicate.not) {
    const value = evaluate(predicate.not, host);
    return value === null ? null : !value;
  }
  if (predicate.value === undefined) {
    return Object.hasOwn(host.flags, predicate.option) ? host.flags[predicate.option] : null;
  }
  return Object.hasOwn(host.values, predicate.option) ? host.values[predicate.option] === predicate.value : null;
}

/** An attribute's path, and its parenthesised tokens or `= value`. */
function parseAttribute(tokens, line) {
  const names = [];
  let index = 0;
  while (index < tokens.length && (tokens[index].kind === 'identifier' || tokens[index].text === '::')) {
    if (tokens[index].kind === 'identifier') {
      names.push(tokens[index].text);
    }
    index += 1;
  }
  const attribute = { path: names.join('::'), line };
  if (tokens[index]?.text === '(') {
    attribute.arguments = tokens.slice(index + 1, skipGroup(tokens, index) - 1);
  } else if (tokens[index]?.text === '=') {
    attribute.value = tokens[index + 1]?.value;
  }
  return attribute;
}

/**
 * `attributes` with every `cfg_attr` replaced by the attributes it applies,
 * each carrying the predicate that applies it as `when`.
 */
function expandAttributes(attributes, when = null) {
  return attributes.flatMap((attribute) => {
    if (attribute.path !== 'cfg_attr' || !attribute.arguments) {
      return [{ ...attribute, when }];
    }
    const [predicate, ...applied] = splitArguments(attribute.arguments);
    const condition = parsePredicate(predicate);
    return expandAttributes(
      applied.map((tokens) => parseAttribute(tokens, attribute.line)),
      when ? { all: [when, condition] } : condition,
    );
  });
}

const isTestAttribute = (attribute) => attribute.path === 'test' || attribute.path.endsWith('::test');

/** The `cfg` predicates that unconditionally apply to an item. */
const conditionsOf = (attributes) =>
  attributes
    .filter((attribute) => attribute.path === 'cfg' && !attribute.when && attribute.arguments)
    .map((attribute) => parsePredicate(attribute.arguments));

/**
 * The tests in one source file and the module files it declares. `scope`
 * holds the conditions the file compiles under and the directory its child
 * modules live in.
 */
export function scanSource(source, { file, conditions = [], directory }) {
  const tokens = tokenize(source);
  const tests = [];
  const modules = [];
  const scopes = [{ conditions, directory, inline: false }];
  let pending = [];
  for (let index = 0; index < tokens.length; ) {
    const token = tokens[index];
    const scope = scopes[scopes.length - 1];
    const following = tokens[index + 1]?.text;
    if (token.text === '#' && (following === '[' || (following === '!' && tokens[index + 2]?.text === '['))) {
      const open = following === '[' ? index + 1 : index + 2;
      const end = skipGroup(tokens, open);
      const attribute = parseAttribute(tokens.slice(open + 1, end - 1), token.line);
      if (following === '!') {
        // An inner attribute applies to the module that encloses it.
        scope.conditions = [...scope.conditions, ...conditionsOf([attribute])];
      } else {
        pending.push(attribute);
      }
      index = end;
      continue;
    }
    const attributes = expandAttributes(pending);
    pending = [];
    if (token.text === '{') {
      scopes.push({ ...scope });
      index += 1;
      continue;
    }
    if (token.text === '}') {
      scopes.pop();
      index += 1;
      continue;
    }
    let keyword = index;
    if (tokens[keyword].text === 'pub') {
      keyword = tokens[keyword + 1]?.text === '(' ? skipGroup(tokens, keyword + 1) : keyword + 1;
    }
    if (tokens[keyword]?.text === 'mod' && tokens[keyword + 1]?.kind === 'identifier') {
      const name = tokens[keyword + 1].text;
      const terminator = tokens[keyword + 2]?.text;
      const nested = [...scope.conditions, ...conditionsOf(attributes)];
      if (terminator === '{') {
        scopes.push({ conditions: nested, directory: path.join(scope.directory, name), inline: true });
        index = keyword + 3;
        continue;
      }
      if (terminator === ';') {
        const custom = attributes.find((attribute) => attribute.path === 'path' && !attribute.when)?.value;
        const candidates = custom
          ? // Outside inline modules a path is relative to the declaring file.
            [path.join(scope.inline ? scope.directory : path.dirname(file), custom)]
          : [path.join(scope.directory, `${name}.rs`), path.join(scope.directory, name, 'mod.rs')];
        modules.push({ name, line: token.line, candidates, conditions: nested });
        index = keyword + 3;
        continue;
      }
    }
    if (attributes.some(isTestAttribute)) {
      let name = index;
      while (name < tokens.length && tokens[name].text !== 'fn') {
        name += 1;
      }
      const always = attributes.some((attribute) => isTestAttribute(attribute) && !attribute.when);
      tests.push({
        file,
        line: tokens[name + 1]?.line ?? token.line,
        name: tokens[name + 1]?.text ?? '?',
        conditions: [
          ...scope.conditions,
          ...conditionsOf(attributes),
          // A test that only `cfg_attr` marks is a test where its predicate holds.
          ...(always
            ? []
            : [{ any: attributes.filter(isTestAttribute).map((attribute) => attribute.when) }]),
        ],
        ignoredWhen: attributes
          .filter((attribute) => attribute.path === 'ignore' && attribute.when)
          .map((attribute) => attribute.when),
      });
    }
    index += 1;
  }
  return { tests, modules };
}

/** The tests of every target that a test build of `crate` compiles. */
export function crateTests(crate, root = ROOT) {
  const tests = [];
  const problems = [];
  const visited = new Set();
  const visit = (file, conditions, directory) => {
    if (visited.has(file)) {
      return;
    }
    visited.add(file);
    const relative = path.relative(root, file).split(path.sep).join('/');
    const result = scanSource(readFileSync(file, 'utf8'), { file: relative, conditions, directory });
    tests.push(...result.tests);
    for (const module of result.modules) {
      const candidates = module.candidates.map((candidate) => path.resolve(root, candidate));
      const found = candidates.find((candidate) => existsSync(candidate));
      if (!found) {
        problems.push(`${relative}:${module.line}: cannot find the file of module \`${module.name}\``);
        continue;
      }
      const ownsDirectory = path.basename(found) === 'mod.rs';
      visit(
        found,
        module.conditions,
        ownsDirectory
          ? path.dirname(found)
          : path.join(path.dirname(found), path.basename(found, '.rs')),
      );
    }
  };
  for (const target of crate.roots) {
    // A crate root is a mod-rs file: its modules sit beside it.
    visit(path.resolve(root, target), [], path.dirname(path.resolve(root, target)));
  }
  return { tests, problems };
}

/** The hosts CI tests `crate` on. */
export const ciHosts = (crate) => ['linux', ...crate.hosts];

/**
 * Where `test` runs: hosts on which it compiles and is not ignored. A host
 * on which that is unknown counts as running it.
 */
export function runningHosts(test, hosts) {
  return hosts.filter((host) => {
    const configuration = HOST_CONFIGURATIONS[host];
    return (
      test.conditions.every((condition) => evaluate(condition, configuration) !== false) &&
      !test.ignoredWhen.some((condition) => evaluate(condition, configuration) === true)
    );
  });
}

/** Problems that keep `crates` from running each of their tests in CI. */
export function verify(crates, root = ROOT) {
  const problems = [];
  const gated = [];
  for (const crate of crates) {
    const { tests, problems: unreadable } = crateTests(crate, root);
    problems.push(...unreadable.map((problem) => `${crate.name}: ${problem}`));
    const hosts = ciHosts(crate);
    for (const test of tests) {
      const everywhere = runningHosts(test, Object.keys(HOST_CONFIGURATIONS));
      if (everywhere.length < Object.keys(HOST_CONFIGURATIONS).length) {
        gated.push(`${crate.name}: ${test.file}:${test.line} ${test.name} runs on ${everywhere.join(', ') || 'no host'}`);
      }
      if (runningHosts(test, hosts).length === 0) {
        problems.push(
          `${crate.name}: ${test.file}:${test.line} \`${test.name}\` runs on none of the hosts CI tests this crate on (${hosts.join(', ')})`,
        );
      }
    }
  }
  return { problems, gated };
}

function main() {
  const list = process.argv.includes('--list');
  const crates = workspaceCrates(readMetadata());
  const { problems, gated } = verify(crates);
  if (list) {
    console.log(gated.length > 0 ? gated.join('\n') : 'no test depends on its host');
  }
  if (problems.length > 0) {
    console.error(
      [
        'Tests that CI never runs:',
        ...problems.map((problem) => `  ${problem}`),
        'Move a test that needs macOS or Windows to rust/crates/platform-tests,',
        'or declare the host in the crate: see CONTRIBUTING.md, "Platform tests".',
      ].join('\n'),
    );
    process.exitCode = 1;
    return;
  }
  console.log(`platform tests: every test of ${crates.length} crates runs on a host CI tests it on`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main();
}
