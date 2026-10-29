## What and why

## How it was tested

List the commands you ran and anything you could not run.

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] regtest demos, if Lightning, Bitcoin Core, webhooks or the sandbox changed

## Checklist

- [ ] No credentials, keys or real funds data
- [ ] State changes are validated and idempotent
- [ ] Docs and the feature matrix match what was actually verified
