# Security Policy

## Supported Versions

We provide security updates for the following versions of OpenHuman:

| Version        | Supported          |
| -------------- | ------------------ |
| Latest         | :white_check_mark: |
| Previous minor | :white_check_mark: |
| Older          | :x:                |

We recommend always running the [latest release](https://github.com/alphahumanxyz/openhuman/releases/latest). OpenHuman is in early beta; older versions may not receive patches.

## Reporting a Vulnerability

We take security seriously. If you believe you have found a security vulnerability, please report it responsibly.

### How to Report

1. **Do not** open a public GitHub issue for security vulnerabilities.
2. Email the maintainers with a clear description of the issue, steps to reproduce, and impact. You can reach us via the contact details listed in the [OpenHuman organization](https://github.com/alphahumanxyz) or repository.
3. Include as much detail as possible (platform, version, configuration) so we can reproduce and triage quickly.

### What to Expect

- We will acknowledge your report as soon as possible (typically within 5 business days).
- We will keep you updated on our assessment and any fix or mitigation.
- We will credit you in our security advisories and release notes (unless you prefer to remain anonymous).

### Scope

We are especially interested in:

- Authentication or authorization bypass
- Data exfiltration or exposure (credentials, messages, user data)
- Remote code execution (frontend, Tauri/Rust backend, or skills runtime)
- Issues in dependency chain (npm, Cargo) that affect our build or runtime
- Platform-specific issues (macOS, Windows, Linux, Android, iOS) that compromise user data or device security

Out-of-scope for this process: general bugs, feature requests, and issues in third-party services we integrate with (e.g., Telegram, Notion) unless they are specific to how OpenHuman uses them.

### Safe Harbor

We support safe harbor for security researchers who report in good faith. We will not pursue legal action or involve law enforcement against you for discovering or reporting vulnerabilities in accordance with this policy.

## Security Practices

- **Credentials**: Desktop uses OS-level credential storage (e.g., macOS Keychain, Windows Credential Manager). We do not store secrets in plain text.
- **Data**: Message content is processed on request and not retained for training or long-term storage.
- **Skills**: Skills run in a sandboxed environment with defined boundaries; we review skill behavior and dependencies where possible.

Thank you for helping keep OpenHuman and its users safe.
