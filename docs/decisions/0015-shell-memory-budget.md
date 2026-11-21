# 0015 — Shell memory budget: anonymous memory, gated on Linux

- **Status:** Accepted
- **Date:** 2026-11-16

## Context

The footprint budgets ([00-overview](../spec/00-overview.md#footprint-budgets)) allowed
the desktop shell 25 MB of resident memory, excluding the webview process. The first
measurement (M2 spike: release build, a bare Tauri 2 window, Ubuntu 24.04 under Xvfb,
WebKitGTK 2.50, 8 s after start) gave:

| Setting | Resident | Anonymous |
|---|---|---|
| Default | 185 MB | 45 MB |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | 149 MB | 33 MB |

On Linux, WebKitGTK's UI side runs inside the shell process. About 100 MB of the resident
memory is shared library pages: Mesa's `libLLVM` (49–61 MB, software GL under Xvfb),
`libwebkit2gtk` (26 MB), JavaScriptCore and GTK. Other processes share these pages, and
a real GPU driver maps different ones. The shell binary itself is 4 MB. The web content
process (301 MB) and the network process (63 MB) are separate and were never part of the
budget.

No setting brings the shell under 25 MB resident, and resident memory mostly measures the
system's libraries, not the shell.

## Decision

- The shell budget counts **anonymous memory** (`Anonymous` in `/proc/<pid>/smaps_rollup`):
  memory the process itself allocated, including WebKitGTK's UI side, excluding shared
  library pages. The budget is **≤ 60 MB**, measured on Linux with a release build, the UI
  kernel and the bundled plugins loaded, one second after the UI connects to the core,
  median of 5.
- `scripts/footprint.sh` checks it on Linux in CI. Resident memory is printed next to it,
  not gated.
- macOS and Windows report the shell's resident memory in CI, not gated, until there are
  numbers to set a budget from.

## Consequences

- The budget follows what the shell and its webview's UI side allocate, which the project
  can influence, rather than the system's shared libraries, which it cannot.
- 60 MB leaves room over the bare window's 33–45 MB for the bridge and the UI kernel's
  effect on the UI process. If the shell grows past it, the breakdown printed by the check
  is the starting point.
- A budget for macOS and Windows is still to be set, from CI numbers.
