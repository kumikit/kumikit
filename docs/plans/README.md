# Plans

Working plans for changes that span several files or steps. A plan lets an agent (or a
person) pick up work in a new session without the earlier conversation.

## When

Write a plan before work that touches several files, crosses crates or packages, or
implements a roadmap step. Skip it for one-file fixes.

## Format

File name: `YYYY-MM-DD-short-title.md`.

```markdown
# Title

- **Goal:** one sentence; link the spec section or roadmap step.
- **Status:** active | done | abandoned

## Steps
- [ ] Step, with the files it touches and how it is verified
- [ ] …

## Notes
Findings, surprises and decisions made along the way (move lasting ones to the spec or an ADR).
```

## Rules

- Keep the checklist current as you work; it is the handoff.
- When done, set the status to `done` and move anything of lasting value into the spec or
  an ADR. Done plans can be deleted once the work has shipped.
