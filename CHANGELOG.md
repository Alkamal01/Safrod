# Changelog

All notable changes are recorded here. The project follows semantic versioning once a release is tagged; nothing is released yet.

## Unreleased

### Added
- Rust workspace for a self-hostable Bitcoin and Lightning payment backend on regtest: core types, PostgreSQL storage with a transactional outbox, LDK Node adapter, Bitcoin Core deposit watcher, signed webhooks with SSRF protection, simulated sandbox, REST API with OpenAPI, daemon and operator dashboard.
- Example merchant app, regtest setup and demo scripts, native and Docker Compose runners, CI workflow.
- Upstream audit, architecture, threat model, API and demo guides, progress report.

### Changed
- The first-iteration Core Lightning settlement prototype moved to `legacy/settlement` and its package was renamed `safrod-settlement`.
- Product name is Safrod throughout.
