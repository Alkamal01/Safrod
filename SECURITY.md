# Security policy

## Supported versions

Safrod is an unaudited prototype that runs on Bitcoin regtest only. Security fixes are made on the latest `main`; no released versions are supported yet. Do not use it with real funds.

## Reporting a vulnerability

Do not report vulnerabilities involving funds, keys, payment integrity, authentication, replay, webhook signing or data exposure in a public issue.

Use GitHub's private vulnerability reporting for this repository (Security tab, "Report a vulnerability"). Include a description, affected code paths, reproduction steps, impact and any suggested mitigation. Please allow reasonable time for triage and a fix before disclosure.

Known limitations that are documented in [`docs/threat-model.md`](docs/threat-model.md) are not vulnerabilities, but improvements to them are welcome as pull requests.
