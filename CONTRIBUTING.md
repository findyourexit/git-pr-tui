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

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
