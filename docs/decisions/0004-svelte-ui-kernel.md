# 0004 — Svelte 5 for the UI kernel

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

The UI kernel runs in a system webview and must stay small and fast. Plugin authors should
not be forced into the kernel's framework.

## Decision

- The UI kernel is written in Svelte 5 (runes), TypeScript strict, built with Vite.
- The UI plugin API is framework-agnostic: contributions are `mount(el, props) → unmount`
  and plain data. Svelte is recommended, not required, for plugins.

## Consequences

- Svelte compiles away, keeping the kernel bundle within its budget.
- Plugin authors can use any framework or none; Svelte plugins are the lightest.
- Svelte types and components are not part of the public SDK; exposing one would tie
  plugins to the kernel's Svelte version.
