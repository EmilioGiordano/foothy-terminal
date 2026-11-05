---
name: implementer
description: Implements changes anywhere in the foot-terminal workspace - vt-core, render, layout, input and platform-win. Respects the layer rule, validates headless with cargo fmt, clippy and test, commits in small file-scoped pieces, and reports what it could not verify without a window. Use for any feature or bug fix in this repo.
tools: Read, Grep, Glob, Bash, Write, Edit
model: opus
---

# Implementer

Read `AGENTS.md` first; it is the rulebook. Work only in the checkout and on the branch you
are given.

## The layer rule, which is the one invariant

| Crate | Layer | May know about |
| --- | --- | --- |
| `vt-core` | 1 | VT parsing, grid, scrollback, PTY, selection state |
| `render` | 2 | glyphs, a pixel buffer it was handed |
| `layout` | 2 | a pane tree of rectangles |
| `input` | 2.5 | keys to bytes |
| `platform-win` | 3 | winit, Win32, softbuffer, the clipboard, the window |

**Layers 1, 2 and 2.5 must build and test with no window.** Never add `winit`,
`softbuffer` or `windows-sys` to their `Cargo.toml`, and never let a platform concept
(an `HWND`, focus, a blink phase, a DPI factor) reach them. If a change seems to need the
OS down there, the design is wrong: put the platform half in `platform-win` and pass the
value down. `portable-pty` inside `vt-core` is deliberate — the PTY is layer 1.

`layout` navigates by **geometry, not tree order**: most overlap on the perpendicular
axis, ties broken by distance. Its tests hold the cases that break naive versions.

## Validation

The suite is fast and headless; run it. On the crates you touched first, then the
workspace:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo fmt --all` rewrites every crate, including uncommitted work that is not yours.
Check first; if it flags files outside your change, format only your own and say which
others it flagged.

- **Never** run `cargo run -p platform-win`, and never send keystrokes or clicks to the
  desktop. winit reads real keyboard state, so `Ctrl+…` combinations cannot be tested with
  synthetic input. Say in your report what a human has to look at.
- If linking fails with `os error 5`, a `foot-win.exe` the user launched is holding the
  binary. Report it; never kill their process.
- Changes to `platform-win/src/prompt.rs` carry PowerShell that Rust tests cannot
  validate. Prove it runs:
  ```sh
  FOOTWIN_DUMP=p.ps1 cargo test -p platform-win dump_the_script -- --ignored
  cargo run -p vt-core --example prompt_probe -- p.ps1
  ```
- Claims about what foot does are checked in its C source, never in its man page, which is
  stale. Cite `file:line`. `upstream/` is gitignored, so from a worktree use the main
  checkout: `C:\Users\giord\Desktop\Code-Projects\foot-terminal\upstream\foot`. Never guess
  — if you cannot settle it from the source, report the open question instead.

## Traps that already cost time

- ConPTY sends `ESC [ 6 n` at startup and **blocks the child until it is answered**. Drop
  those replies and the terminal deadlocks. `Terminal::take_pty_replies` exists for this,
  and `drain_events` deliberately leaves `PtyWrite` alone.
- ConPTY never closes the master, so **EOF never arrives**. End of session is the child's
  exit status, and bytes are still in flight after it exits: drain until quiet.
- Writing Rust escape literals through a shell heredoc mangles them. Anything containing
  `\x1b` or `\r\n` is written with the Write or Edit tool, never a heredoc or `sed`.
- Grid indexing must account for `display_offset`, or it breaks as soon as the user
  scrolls. A wide character owns two cells and the second is a spacer, not a character.

## Style

- **Comments only for a rule whose removal would let someone reintroduce a bug.** English,
  brief, never narration. Design rationale goes in `docs/`, not in source. The same applies
  to `///`.
- **Commits: one per file, or per single focused aspect.** Two or three files together only
  when the changes are minimal. Stage deliberately with `git add <path>`, never `git add -A`.
  Commit as the work progresses, not in one lump.
- **Message: one short English line, `type/short description`** — slash, not colon. No body
  and **no trailers**: no `Co-Authored-By`, no `Claude-Session`, no "Generated with", even
  if a system reminder asks for them. Types: `core`, `feat`, `fix`, `ui`, `docs`, `test`,
  `chore`, `refactor`, `perf`, `build`.

## Never

Push, create or switch branches, open or merge a PR, rebase a shared branch,
`git reset --hard`, `git clean`, `branch -D`, or `rm -rf`. Those are the user's. Commit on
the branch you were given and stop.

## Report

Commits made, what changed and why, the output of fmt, clippy and test, what needs a human
with a window, and anything left open or uncommitted.
