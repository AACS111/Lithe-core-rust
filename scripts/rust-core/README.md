# Rust Core CI scripts

The [Rust Core workflow](../../.github/workflows/rust-core.yml) runs these
scripts; [`CONTRIBUTING.md`](../../CONTRIBUTING.md) explains the rules behind
them. Test the Node.js scripts with `node --test scripts/rust-core/*.test.mjs`.

## Affected crates

`node scripts/rust-core/affected-crates.mjs [--base <rev>] [--all] [--github]`
prints the crates that the changes since the merge base of `<rev>` (default
`origin/main`) and the working tree can affect, and the arguments each CI host
tests them with. It reads `cargo metadata --no-deps`, so it needs no network.
`--all` selects every crate; `--github` also writes the selection to
`$GITHUB_OUTPUT`. A crate that must also be tested on macOS or Windows declares
those hosts, and the features its tests need, under
`[package.metadata.lithe-ci]`.

## Platform test placement

`node scripts/rust-core/verify-platform-tests.mjs [--list]` reads the source of
every target a test build compiles, follows module declarations, and fails when
a test is compiled out by `cfg`, or ignored by `cfg_attr(…, ignore)`, on every
host CI tests its crate on. `--list` also prints every test that runs on only
some hosts. Options it cannot decide, such as features, count as enabled.

## Comment rules

Run `scripts/rust-core/verify-rust-core-comments.sh [crate-directory ...]` from
any directory. It checks module documentation, English comments, unsafe API
safety sections, exported Rustdoc, and intra-doc links. The textual checks
always cover every crate under `rust/crates/`; Rustdoc builds the crates whose
directories are given, or every crate without arguments. Every crate must pass
`missing_docs`; there is no exemption list.

The small `comment-lexer` helper uses pinned `rustc_lexer` tokens to distinguish
comments from ordinary, raw, and byte string literals. It does not parse source
with delimiter regular expressions or exempt Unicode test data. Run its tests
with `cargo test --locked --manifest-path scripts/rust-core/comment-lexer/Cargo.toml`.

The gate uses `CARGO_TARGET_DIR` when supplied. Otherwise it writes only to this
checkout's `.artifacts/rust-core-checks/`. Cargo state and generated Rustdoc stay
local to the checkout, are ignored by Git, and must not be shared with another
worktree or installed with the application.
