# Rust toolchain bump

Nothing proposes Rust releases automatically: Dependabot ignores the toolchain
(#1002). **Bump stable by hand every 2–3 stable releases, or soon after a Rust
security release.** Do it as its own PR. The 1.95.0 → 1.99.1 bump (#1027)
needed lint fixes, a dependency bump, a snapshot update and CI disk fixes, so
plan for some fallout.

## Stable toolchain checklist

These are the known places. Search for the old version as a backstop:
`rg --hidden -F <old> -g '!*.lock' -g '!.git' -g '!docs/' -g '!qa/'`.

- `codex-rs/rust-toolchain.toml` (`channel`).
- Workflows:
    - every `dtolnay/rust-toolchain@<sha> # <version>` step. With no
  `toolchain:` input the SHA selects the compiler, so use the commit at the
  head of the action's `<version>` branch, not just a new comment.
  Dependabot ignores this action on purpose;
    - explicit `toolchain: "<version>"` inputs and `cargo +<version>` commands
  (`rusty-v8-release.yml`, `v8-canary.yml`), and the version named in
  `cargo-deny.yml`'s comment.
- `MODULE.bazel`: `toolchains.toolchain(version = ...)` and the
  `rust_windows_x86_64_gnullvm_stable` set's `versions`.
- Bazel lockfile and checksums (#1032), in this order:
    1. `just bazel-lock-update`. The checksum script never rewrites the
       lockfile and fails while it is stale.
    2. `python3 .github/scripts/rust_toolchain_bazel_sha256s.py` regenerates
       the `RUST_TOOLCHAIN_SHA256S` block in `MODULE.bazel`. Without it the
       gnullvm toolchain downloads have no checksum.
    3. `just bazel-lock-update` again, then `just bazel-lock-check` and the
       script's `--check`. (A PR unit test fails if a declared version has no
       pins; the nightly Bazel job runs the full `--check`.)
- CI and dev images: `.devcontainer/Dockerfile.secure` (`RUST_TOOLCHAIN`),
  `.devcontainer/devcontainer.secure.json`,
  `codex-rs/scripts/setup-windows.ps1` (`$toolchain`) and
  `.codex/environments/environment.toml` (`cargo +<version>`).
- Maintainers' machines (outside the repo):
    - the Mac dev-build scripts `.codex-work/integration-build-*/dev-build.sh`
  and `dev-build-online.sh` (under the Corbanu folder on the CorbanuDrive
  volume) put `$RUNTIME/rustup/toolchains/<version>-aarch64-apple-darwin/bin`
  first on `PATH`. Install the new toolchain there first, then change both
  scripts;
    - the worker notes that name the version (`.codex-work/workers-*/`, the
  shared worker rules and the glitch section);
    - glitch and the RTX box pick up the new toolchain from
  `rust-toolchain.toml` at their next build.

Then:

- `just fmt`, and clippy with `-D warnings` on macOS and on Linux (glitch),
  with and without `developer-accounting`. Fix new lints in the code;
  allow one only with a `reason`.
- Rerun snapshot tests: new compilers can change `Debug` output.
- Dispatch `rust-ci-full` and the nightly cold Bazel jobs on the branch
  before merging. A cold Bazel cache after a bump has filled the runners'
  disk before.

## argument-comment-lint nightly

The lint (`tools/argument-comment-lint`) builds on `rustc_private`, so it pins
a nightly, not stable. Move it when it blocks new std APIs or every few stable
bumps (#1030). The date appears in:

- `tools/argument-comment-lint/rust-toolchain`, `wrapper_common.py`,
    `src/bin/argument-comment-lint.rs` (library file names) and the README;
- `rust-ci.yml`, `rust-ci-full.yml` and `rust-release-argument-comment-lint.yml`
    (library file names too);
- the three `nightly/<date>` entries in `MODULE.bazel`; then update the
    lockfile and checksums as above;
- any `#[allow(..., reason = "...")]` in `codex-rs` that names the nightly
    (`rg -F nightly-<date> codex-rs`): remove it once the nightly moves.

Picking the nightly:

- Use the nightly of a rust-clippy "rustup" sync commit (its
    `rust-toolchain.toml`) and pin `clippy_utils` to that commit.
- Use a dylint version that builds on that nightly. Pin `dylint_linting` and
    `dylint_testing` exactly, and match the workflows' `CARGO_DYLINT_VERSION`
    and `DYLINT_LINK_VERSION`.
- The whole workspace must compile and pass on it, on Linux and macOS:
    `python3 tools/argument-comment-lint/run.py -- --keep-going`. Nightlies from
    2026-09-01 on break `allocative` 0.3.6 (its `impl Allocative for !`
    conflicts with `Infallible`), so the lint stays on 2026-08-23 until that is
    fixed.
