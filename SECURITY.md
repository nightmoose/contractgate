# Security Policy

## Supported Versions
ContractGate is rapidly evolving. We support security updates for the latest stable release only.

## Reporting a Vulnerability
We take security seriously. Please report vulnerabilities privately.

**Preferred**: Use GitHub's private vulnerability reporting (recommended).
**Alternative**: Email security@contractgate.dev (or alex.suarez@nightmoose.com) with:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

We aim to acknowledge reports within 48 hours and provide a fix timeline within 7 days.

## Disclosure Policy
- We follow coordinated vulnerability disclosure.
- Public disclosure only after a fix is released (or 90 days, whichever is sooner).
- We credit reporters in release notes unless anonymity requested.

## Security Features & Practices
- Secret Scanning + Push Protection (GitHub)
- Dependabot dependency updates
- CodeQL static analysis
- `cargo audit` + `cargo deny` in CI
- Signed commits required on `main`
- Branch protection rules

## Additional Hardening
See `docs/SECURITY.md` (forthcoming) for supply-chain, Rust-specific, and runtime security details.

Thank you for helping keep ContractGate secure.
