# 0026 — First-party plugins live in their own repositories

- **Status:** Accepted
- **Date:** 2026-11-20

## Context

M5 adds the first plugins that are not part of the framework: agent providers, git,
GitHub and scheduled prompts. Kept in this repository they would share its CI, its release
and its review, and nothing would show that they need only the public SDKs (AGENTS.md:
first-party plugins use only public SDKs). Third parties will build plugins outside this
repository; the first-party ones should be built the same way.

## Decision

- From M5 on, new first-party plugins live in repositories under `github.com/kumikit`
  named `kumikit-plugin-<name>`, grouped where they ship together:
  `kumikit-plugin-agents` (every agent provider), `kumikit-plugin-git` (the git plugin and
  its panel), `kumikit-plugin-github`, `kumikit-plugin-schedules`.
- They depend on the SDKs by git, pinned to a commit of this repository (crates.io and
  npm releases of the SDKs are a later decision), and their CI builds `kumikit` from the
  same pin for end-to-end tests.
- This repository keeps the framework, the preinstalled plugins (`provider-acp`,
  `ui-chat`, `ui-settings`, `theme-default`), the templates and the test plugins that
  prove each general API. Its exit tests use those test plugins.
- Each plugin repository publishes its packages as GitHub releases and to the registry
  through the normal pull requests.

## Consequences

- The SDKs are exercised from outside, as third parties will use them; a missing public
  API shows up as a build failure there, not as a private import.
- A change to a general API reaches the plugins only when their pin moves; plugin CI
  catches breakage then.
- Some end-to-end coverage (a real git plugin in the desktop app) moves to the plugin
  repositories, against a pinned core rather than the code under review here.
