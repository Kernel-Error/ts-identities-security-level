<!--
⚠️  Please do not attach exported TeamSpeak identity .ini files anywhere
    in this PR (description, comments, screenshots, test data). They
    contain private signing material. Use a self-generated synthetic
    identity for reproducers — the existing tests show how.
-->

## Summary

<!-- 1–3 sentences on what this PR does and the user-visible effect. -->

## Changes

<!-- Bulleted, high-level. Mention any touched crate. -->

## Test plan

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --release` (no new warnings)
- [ ] `cargo test --workspace --release`
- [ ] If the kernel changed: GPU/CPU parity test still passes
- [ ] If any user-facing string changed: `cargo run -p xtask --release -- msgfmt` succeeds and the affected `po/*.po` files were updated

## Linked issues

<!-- e.g. Closes #12 -->

## Notes for reviewers

<!-- Anything non-obvious about the approach, trade-offs, or follow-up work. -->
