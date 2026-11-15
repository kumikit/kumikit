# Security policy

Kumikit runs coding agents and third-party plugins on your machines, so security reports
are taken seriously.

## Reporting a vulnerability

Please report privately through GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
on the affected repository in the `kumikit` organization. Do not open a public issue.

Include what you found, how to reproduce it, and the impact you expect. You will get an
acknowledgement within a few days.

## Scope

The kernel, the plugin host and its sandbox, the protocol and pairing, the desktop shell,
first-party plugins and the plugin registry. Vulnerabilities in third-party plugins should
go to their authors; a way for a plugin to escape its permissions is in scope here.

The threat model is in [docs/spec/11-security.md](docs/spec/11-security.md).
