# Rust toolchain bump

Nothing proposes Rust releases automatically: Dependabot ignores the toolchain
(#1002). **Bump stable by hand every 2–3 stable releases, or soon after a Rust
security release.** Do it as its own PR. The 1.95.0 → 1.99.1 bump (#1027)
needed lint fixes, a dependency bump, a snapshot update and CI disk fixes, so
plan for some fallout.

## Stable toolchain checklist

Search for the old version (`rg -F 1.99.1`, with `-g '!*.lock'`) and update
every hit. The places are:

- [ ] `codex-rs/rust-toolchain.toml` (`channel`).
- [ ] Every workflow's `dtolnay/rust-toolchain@<sha> # <version>` step. The
      SHA selects the compiler, so use the action commit for the new version's
      tag, not just a new comment. Dependabot ignores this action on purpose.
- [ ] `MODULE.bazel`: `toolchains.toolchain(version = ...)` and the
      `rust_windows_x86_64_gnullvm_stable` set's `versions`.
- [ ] Bazel checksums: run `python3 .github/scripts/rust_toolchain_bazel_sha256s.py`
      to regenerate the `RUST_TOOLCHAIN_SHA256S` block in `MODULE.bazel`
      (#1032). Without this, the gnullvm toolchain downloads have no checksum.
      The PR unit test fails if a declared version has no pins; the nightly
      Bazel job runs the full `--check`.
- [ ] `MODULE.bazel.lock`: `just bazel-lock-update`.
- [ ] CI and dev images: `.devcontainer/Dockerfile.secure` (`RUST_TOOLCHAIN`),
      `.devcontainer/devcontainer.secure.json`,
      `codex-rs/scripts/setup-windows.ps1` (`$toolchain`) and
      `.codex/environments/environment.toml` (`cargo +<version>`).
- [ ] Outside the repo: the Mac dev-build scripts (`dev-build.sh` and
      `dev-build-online.sh` in the integration-build folder) put
      `$RUNTIME/rustup/toolchains/<version>-aarch64-apple-darwin/bin` first on
      `PATH`. Install the new toolchain there first, then change both scripts.
      Update the worker environment notes that name the version (the shared
      worker rules and the glitch notes). glitch and the RTX box pick up the
      new toolchain from `rust-toolchain.toml` at their next build.

Then:

- [ ] `just fmt`, and clippy with `-D warnings` on macOS and on Linux (glitch),
      with and without `developer-accounting`. Fix new lints in the code;
      allow one only with a `reason`.
- [ ] Rerun snapshot tests: new compilers can change `Debug` output.
- [ ] Dispatch `rust-ci-full` and the nightly cold Bazel jobs on the branch
      before merging. A cold Bazel cache after a bump has filled the runners'
      disk before.

## argument-comment-lint nightly

The lint (`tools/argument-comment-lint`) builds on `rustc_private`, so it pins
a nightly, not stable. Move it when it blocks new std APIs or every few stable
bumps (#1030). It is set in `tools/argument-comment-lint/rust-toolchain`,
`wrapper_common.py`, the README, `rust-ci.yml`, `rust-ci-full.yml`,
`rust-release-argument-comment-lint.yml` (library file names too) and the three
`nightly/<date>` entries in `MODULE.bazel`. Then rerun the checksum script and
`just bazel-lock-update`.

- Pick the nightly of a rust-clippy "rustup" sync commit (its
  `rust-toolchain.toml`) and pin `clippy_utils` to that commit.
- Pick a dylint version that builds on that nightly. Pin `dylint_linting` and
  `dylint_testing` exactly, and match the workflows' `CARGO_DYLINT_VERSION`
  and `DYLINT_LINK_VERSION`.
- The whole workspace must compile on it:
  `python3 tools/argument-comment-lint/run.py -- --keep-going`. Nightlies from
  2026-09-01 on break `allocative` 0.3.6 (its `impl Allocative for !`
  conflicts with `Infallible`); move past 2026-08-23 only once it is fixed.
