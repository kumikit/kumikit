# 0001 — Record architecture decisions

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

Kumikit will be developed largely with coding agents. An agent sees the code and the docs
but not the conversations in which decisions were made, so it may "fix" a deliberate
choice or reopen a settled question.

## Decision

Significant decisions are recorded as short ADRs in `docs/decisions/`, following
[the README](README.md). The spec in `docs/spec/` says *what*; ADRs say *why*.

## Consequences

- Before changing an area, read its spec section and the ADRs it links.
- Reversing a decision means a new ADR, not an edit of the old one.
