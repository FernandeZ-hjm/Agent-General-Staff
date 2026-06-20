# Contributing to AGS

Thanks for your interest in contributing to Agent General Staff.

## Getting Started

```bash
git clone https://github.com/FernandeZ-hjm/Agent-General-Staff.git
cd Agent-General-Staff
cargo build
cargo test --all
```

## Before Submitting a PR

Run the local verification gate:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test --all
ags verify --scope local
```

All four must pass. CI runs the same checks on three platforms (Linux, macOS,
Windows).

## Code Style

- Follow existing conventions in the file you're editing.
- Run `cargo fmt` before committing.
- Keep changes focused: one logical change per PR.

## Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/) style:

```
feat(kernel): add rollback stage to runner
fix(doctor): handle missing suite.yaml gracefully
docs: update README for 2.7 architecture
```

## License

By contributing, you agree that your contributions will be licensed under the
GNU General Public License v3.0 only (GPL-3.0-only), the same license as the
project. See `LICENSE` for the full text.

## Reporting Issues

- Bugs: use the [bug report template](https://github.com/FernandeZ-hjm/Agent-General-Staff/issues/new?template=bug_report.yml)
- Features: use the [feature request template](https://github.com/FernandeZ-hjm/Agent-General-Staff/issues/new?template=feature_request.yml)
- Security: see `SECURITY.md`
