# Security Policy

## Supported versions

Security fixes are applied on a best-effort basis to the default development
branch of this repository. There is no long-term support channel yet.

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Prefer one of:

1. **GitHub Security Advisories** — open a private advisory on the
   [CortexLM/cortex-code](https://github.com/CortexLM/cortex-code) repository
   (Security → Advisories → Report a vulnerability), if available on the repo.
2. **Maintainer contact** — if advisories are unavailable, contact the
   repository owners through GitHub (private maintainers channel / email listed
   on the org profile) and mark the report as security-sensitive.

Include:

- A clear description of the issue and impact
- Steps to reproduce or a proof of concept
- Affected version / commit if known
- Any suggested fix (optional)

We will acknowledge receipt when possible and work on a fix before any public
disclosure. Please give a reasonable window for remediation before publishing
details.

## Scope notes

- Local loopback server defaults (`127.0.0.1`, reserved port range) are
  intentional; binding non-loopback hosts is out of product scope.
- Credential files (`auth.json`) are expected to be mode `0600`; please report
  accidental logging or leakage of secrets.
- Do not test destructive payloads against third-party systems you do not own.
