# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Rust workspace building a fast, light terminal emulator for Windows with multiplexing (splits), modelled on [foot](https://codeberg.org/dnkl/foot). It started as a research repo, so **`docs/` carries the reasoning and `upstream/` carries foot's real C source** (cloned, gitignored) as ground truth.

When a claim about foot matters, read `upstream/foot/` rather than its documentation — the man page is stale in places (e.g. it still documents Solarized defaults while `config.c:33` ships `242424`).

## Commands

```sh
cargo run -p platform-win          # the terminal
cargo test                         # whole workspace
cargo test -p layout               # one crate
cargo test -p render beam_cursor   # one test by name substring
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Diagnostics that need no window:

```sh
cargo run -p vt-core --bin spike -- "dir"        # pump a command, dump the grid as text
cargo run -p platform-win --bin keytest -- win32 # what actually reaches PSReadLine
cargo run -p render --example coverage           # which glyphs the font has
```

Runtime tuning, no rebuild: `FOOTWIN_FONT`, `FOOTWIN_FONT_PX`, `FOOTWIN_PAD`, `FOOTWIN_FG`, `FOOTWIN_BG`, `FOOTWIN_ALPHA`, `FOOTWIN_BLINK`, `FOOTWIN_PASTE_WARN`.

## Architecture: the layer rule

This is the one thing to understand before changing anything.

| Crate | Layer | Owns |
|---|---|---|
| `vt-core` | 1 | VT parser, grid, scrollback, PTY, selection state |
| `render` | 2 | Glyph cache, grid → pixel buffer |
| `layout` | 2 | Pane tree: split, close, resize, directional navigation |
| `input` | 2.5 | Key → bytes, win32-input-mode, paste safety |
| `platform-win` | 3 | winit, softbuffer, Win32 clipboard and window chrome |

**Layers 1, 2 and 2.5 must build and test without opening a window.** No `winit`, no `windows-sys`, no pixels beyond a rectangle they were handed. This is what keeps a second platform additive instead of a rewrite, and it is why ~90% of the test suite runs headless. If a change needs the OS inside those crates, the design is wrong — push the platform part up to `platform-win` and pass values down.

Two consequences worth knowing:

- `layout` uses **geometry, not tree order**, for directional navigation. Tree order does not match what the eye expects; the rule is "most overlap on the perpendicular axis, ties broken by distance". See its tests for the cases that break naive implementations.
- `vt-core` exposes a **read path** (`line_text`, `screen_text`, `is_selected`) on purpose, not just a write path. `docs/05` §3.1 explains why: a screen reader provider needs range-addressable reads, and retrofitting that later is a refactor.

`docs/02` §3 lists ten performance rules taken from foot's source. The ones already load-bearing here: bounded work per event-loop turn, and idle costs 0% CPU (the PTY reader wakes the loop through an `EventLoopProxy` instead of polling; the cursor blink timer only arms while focused).

## Conventions

**Comments.** Only when a rule is important, relevant, and not already obvious from the code — something whose removal would let someone reintroduce a bug. Always English, brief. Do not narrate.

**Commits.** One commit per file, or per single focused aspect. Two or three files together only when the changes are minimal, and rarely. Never group a large set of files into one commit. Split substantial work into several small meaningful commits and commit as the work progresses, not at the end.

Format: one short English line, `type/short description`. No body, no trailers.

```
core/add enhanced validation logic
feat/add font zoom support
fix/ctrl+backspace inserting a literal ^W
ui/beam cursor blinking at the system rate
```

Types in use: `core`, `feat`, `fix`, `ui`, `docs`, `test`, `chore`, `refactor`, `perf`, `build`.

**Branches.** English, descriptive, self-explanatory (`phase-0`, not `fase-0`).

## Gotchas that already cost time

- **ConPTY sends `ESC [ 6 n` on startup and blocks the child until answered.** Drop those replies and the terminal deadlocks with four bytes read. `Terminal::take_pty_replies` exists for this; `drain_events` deliberately leaves `PtyWrite` alone so it cannot eat them.
- **ConPTY never closes the master when the child exits**, so EOF never arrives. End of session on Windows is the child's exit status, and after it exits the pipe still has bytes in flight — drain until quiet or the tail is lost.
- **Modifier keys cannot be tested with synthetic input.** winit reads real keyboard state, which `PostMessage` does not update, so `Ctrl+…` combinations need a human. Non-modifier paths (middle-click paste, mouse wheel, plain keys) can be driven with `PostMessage` plus `PrintWindow`, which also avoids stealing focus.
- **Writing Rust escape literals through a shell heredoc mangles them.** `\x1b` and `\r\n` arrive as raw control bytes in the source. Use the Write/Edit tools for anything containing escapes.

## Reference docs

`docs/01` foot internals with file:line citations · `docs/02` the design doc, layers, performance rules, scope decisions · `docs/03` links · `docs/04` measurements, including this project's own baseline · `docs/05` behaviour and accessibility decisions, and the terminal-vs-shell split that explains why history and line editing are not ours to implement.
