# Security policy

[简体中文](SECURITY.zh-CN.md)

Webtop Manager controls a local Docker daemon and must be treated as
security-sensitive software. Please do not disclose a vulnerability, token,
password, private template, host path, or diagnostic archive in a public issue.

## Supported versions

Security fixes are provided for the latest `1.x` GitHub release. Older minor
releases, pre-release builds, and unversioned source snapshots are unsupported.

## Reporting a vulnerability

Use the repository's **Security > Advisories > Report a vulnerability** flow so
the report remains private. Include the affected version, operating system,
Docker version, reproduction steps, impact, and the smallest redacted evidence
needed to understand the issue.

Do not attach `.wtmpl` files, application data, database files, credentials, or
unredacted controller logs. If a real secret was exposed, revoke or rotate it
before sending the report.

Maintainers will make a best effort to acknowledge a report within seven days.
Disclosure timing will be coordinated after a fix and upgrade path are ready.

For the product's trust boundaries and security guarantees, see
[docs/security.md](docs/security.md).
