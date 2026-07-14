## Summary

<!-- What does this PR do and why? -->

## Checklist

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets --all-features` passes
- [ ] `RUSTFLAGS="-D warnings" cargo test --all` passes
- [ ] `ags verify --scope local` passes
- [ ] Release-facing changes pass `ags verify --scope release`
- [ ] Task-card fixtures (if changed) validated with `bash scripts/validate.sh`
