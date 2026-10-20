## Summary

Describe the problem and the focused solution.

## Related issue

Closes #

## Validation

- [ ] Relevant tests pass.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo test --locked` passes.
- [ ] `cargo clippy --locked --all-targets -- -D warnings` passes.
- [ ] A release wheel builds with `--locked` when package behavior changes.
- [ ] Tests run against the installed wheel when package behavior changes.
- [ ] Converted outputs reopen when conversion changes.

## Review checklist

- [ ] Public documentation and the active specification are synchronized.
- [ ] API compatibility and known fidelity limitations are documented.
- [ ] `Cargo.lock`, license inventories, and `NOTICE` are updated when required.
- [ ] No prohibited engine/UI/CLI/MCP/networking/OCR/JavaScript/print/signing component was added.
- [ ] No generated build artifacts, wheels, caches, native binaries, or fixtures with restricted data are committed.
