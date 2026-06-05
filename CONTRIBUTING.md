# Contributing to gprr

Thanks for your interest in contributing! Here's how to get started.

## Getting Started

1. Fork and clone the repository.
2. Install a recent [Rust toolchain](https://rustup.rs/) (the crate's minimum
   supported version is pinned in `Cargo.toml` via `rust-version`).
3. To *run* `gprr` you also need the [`gh` CLI](https://cli.github.com/)
   authenticated with `repo` scope (`gh auth login --scopes repo`) and `git`
   on your `PATH`. The test suite does **not** require either — it runs fully
   offline against recorded fixtures.
4. Confirm everything works:
   ```bash
   cargo test --all-targets
   ```

## Development Workflow

Before opening a pull request, make sure the same checks CI runs pass locally:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo test --doc
```

Many tests are [`insta`](https://insta.rs/) snapshots. When you intentionally
change rendered output, review and accept the new snapshots with:

```bash
cargo insta review
```

## Commit Messages

This project follows [Conventional Commits](https://www.conventionalcommits.org/)
(`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, …). Keep the subject
imperative and scoped, e.g. `feat(ui): add side-by-side diff toggle`.

## Pull Requests

- Keep changes focused. One logical change per PR.
- Add tests for new functionality where practical.
- Make sure existing tests still pass.
- Follow the existing code style (`cargo fmt` enforces formatting).

## Reporting Issues

Open an issue using one of the [issue templates](.github/ISSUE_TEMPLATE).
Please include what you expected to happen, what actually happened, steps to
reproduce, and your OS, terminal emulator, `gh --version`, and Rust version.

## Releasing (maintainer only)

Releases are driven by tags. Pushing a `v*` tag runs [`.github/workflows/release.yml`](.github/workflows/release.yml), which:

1. cross-compiles release binaries for macOS (arm64/x86_64), Linux (`x86_64`/`aarch64`), and Windows (`x86_64`);
2. packages archives + `checksums.txt` and publishes a GitHub Release whose notes are taken from the `[Unreleased]` section of [`CHANGELOG.md`](CHANGELOG.md);
3. promotes that changelog section to the new version; and
4. updates the `gprr` formula in the [`findyourexit/homebrew-tap`](https://github.com/findyourexit/homebrew-tap) tap (requires a `HOMEBREW_TAP_TOKEN` repository secret).

```bash
# 1. Land your changes under "## [Unreleased]" in CHANGELOG.md
# 2. Bump the version in Cargo.toml, then tag and push
git tag v0.2.0
git push origin v0.2.0
```

Linux targets are built with [`cross`](https://github.com/cross-rs/cross); see [`Cross.toml`](Cross.toml) for the container setup (it provisions CMake so the `aws-lc-rs` crypto backend compiles).

## Recording demo assets (maintainer only)

The README GIFs are produced with [`vhs`](https://github.com/charmbracelet/vhs) from the tapes in [`tapes/`](tapes/), recorded against `gprr --demo` so they are deterministic and contain no real GitHub data.

```bash
brew install vhs                       # also pulls ttyd + ffmpeg
cargo build --release                  # the tapes expect ./gprr on $PATH
cp target/release/gprr /tmp/gprr-demo/   # (tapes set a clean XDG/PATH env)
vhs tapes/demo.tape                     # → assets/demo.gif
vhs tapes/demo-dashboard.tape
vhs tapes/demo-review.tape
```

## Recording API fixtures (maintainer only)

Test fixtures under `tests/fixtures/api/` capture live GitHub API responses so the integration suite can run without network access. CI never re-records — only maintainers run the script manually after the GitHub API shape changes or when a captured fixture grows stale.

```bash
gh auth login --scopes repo          # one-time
scripts/record_fixtures.sh           # captures against cli/cli#1 by default
```

Override the source repo or PR via env vars:

| Env var | Default | Purpose |
|---|---|---|
| `GPRR_FIXTURE_REPO` | `cli/cli` | `owner/name` of the source repo |
| `GPRR_FIXTURE_PR` | `1` | PR number to capture (use one with reviews + checks) |
| `GPRR_FIXTURE_DIR` | `tests/fixtures/api` | Output directory |

Commit the regenerated fixtures in a dedicated commit so the diff review stays focused.

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
