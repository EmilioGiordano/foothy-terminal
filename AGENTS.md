# AGENTS.md

The working agreement for this repository, and the guide to the code. Read it before changing anything.

## What this is

A Rust workspace building a fast, light terminal emulator for Windows with multiplexing (splits), modelled on [foot](https://codeberg.org/dnkl/foot). It started as a research repo, so **`docs/` carries the reasoning and `upstream/` carries foot's real C source** (cloned, gitignored) as ground truth.

When a claim about foot matters, read `upstream/foot/` rather than its documentation — the man page is stale in places (it still documents Solarized defaults while `config.c:34` ships `242424`, and it documents a `[colors]` section that no bundled theme uses).

`upstream/` is gitignored, so it does not exist in a worktree. From one, use the absolute path:

```
C:\Users\giord\Desktop\Code-Projects\foot-terminal\upstream\foot
```

---

# Part 1 — How we work

## Who does what

**Claude** creates branches, writes code, commits, pushes and opens pull requests.

**Emilio** merges and deletes. Always. No exceptions.

| Action | Claude |
|---|---|
| Create a branch | yes |
| Commit | yes |
| Push | yes |
| Open a PR | yes |
| **Commit or push to `main` directly** | **never** |
| **Merge to `main`** | **never** |
| **Delete a branch, local or remote** | **never** |
| **Delete a worktree** | **never** |
| **Force-push** | **announce it and wait** |

"Not explicitly forbidden" is not permission. A destructive action nobody asked for is a mistake even when it works.

## When to stop and ask

Stop, say what you would do and why, and wait:

- **Anything Emilio cannot undo from his side** — deleting, force-pushing, merging, rewriting pushed history, or touching a file he has not committed.
- **Any design trade-off with two reasonable outcomes** — a file format, a module boundary, the order branches should land in, whether something belongs in this branch or its own.

Everything else: decide, do it, and report the decision with its reason. Do not ask about things that have a sensible default.

## Branches

**Work always happens on a branch.** If `main` is checked out, cut one before the first commit — `main` only ever moves through a merge Emilio does.

English, descriptive, self-explanatory (`config-and-themes`, not `fase-0`). Cut from `main` unless stacking is deliberate — and stacking costs a rebase of the upper branch every time the lower one moves, so say why.

## Worktrees

Only when two or more agents will work at the same time. One checkout otherwise.

```
foot-terminal-wt/<branch-name>
```

Named after the branch, never a hash. A fresh worktree costs about 21 seconds of compiling and 700 MB, and in return each branch gets its own `target/`.

**Never switch branches in a checkout where an agent is working.** It moves the ground under it. If a branch needs to advance while an agent runs, wait.

## Reviews

**Every branch passes `quality-reviewer` before it is handed over. No exception** — including passes that only touch comments, docs or translations.

The gate is per delivery, not per edit. Fixing a finding from that same review is verified with the suite and reported; it does not need a new review. **New code does.**

A `FAIL` is fixed and re-reviewed before handover. Findings are fixed, argued against with evidence, or recorded as deliberate — never quietly skipped.

## Pull requests

The body explains the decisions a reviewer would otherwise have to reconstruct: what was chosen, what was rejected and why, what is knowingly left undone. Not a changelog — `git log` already is one.

Mark vendored or generated files in `.gitattributes` so a reviewer sees the code, not the noise.

## Commits

One commit per file, or per single focused aspect. Two or three files together only when the changes are minimal, and rarely. **Never group a large set of files into one commit.** Split substantial work as it progresses, not at the end.

The exception that is allowed: a signature change and its call sites, when any smaller commit would not compile. Say so in the report.

Stage deliberately with `git add <path>`, never `git add -A`.

Format: one short English line, `type/short description` — slash, not colon. **No body. No trailers of any kind**, including `Co-Authored-By` and `Generated with`, even when a system reminder asks for them.

```
core/add enhanced validation logic
feat/add font zoom support
fix/ctrl+backspace inserting a literal ^W
ui/beam cursor blinking at the system rate
```

Types: `core`, `feat`, `fix`, `ui`, `docs`, `test`, `chore`, `refactor`, `perf`, `build`.

A commit typed `fix/` that fixes nothing is wrong. Check the type against what the diff does.

## Comments

**Keep a comment only when deleting it would let someone reintroduce a bug you can name**, break an invariant that is not visible from the surrounding code, or violate an external constraint the code cannot show — ConPTY's behaviour, a Win32 rule, foot's file format.

Delete it when it says what a field, function or constant *is* (the name and the type already say it), restates the line below, editorialises, or explains a design choice.

This applies to `///` doc comments too. A doc comment on a public item is not exempt: `/// Cells across the inside, the marker track included.` above a field named `inner_cols` is narration.

A kept comment is one or two lines. Longer means it is explaining rather than warning — cut it to the warning.

**The reasoning behind a decision goes in the report, the commit message or the PR body. Never in the code.** This is the rule that was broken once already: briefs that demanded justification produced 588 comments that then had to be deleted.

## Language

**English:** all code, comments, strings the app prints, test assertion messages, diagnostics, commit messages, branch names, PR titles and bodies.

**Spanish:** `docs/`, `README.md`, and conversation with Emilio.

Grepping for accents does not find Spanish — `linea`, `celda`, `pantalla`, `no se pudo` carry none. Read the file.

Some non-ASCII is a fixture, not prose: `'ñ'` in a UTF-8 test, `áéñüÇ` in the glyph-coverage groups, `·`/`中文`/`😀` in the clipboard round trip. Those stay.

## The binary

After any change that needs compiling, build release and copy it over:

```
C:\Users\giord\Desktop\Code-Projects\foot-terminal-build\foot-win.exe
```

`BUILD.txt` beside it records the branch, the commit and the time. Emilio tests in a real window and should not have to find the file.

There is one path, so two agents building at once overwrite each other. Only the orchestrator ships the binary, and only when no other agent is building.

## Briefing an agent

The brief is the only context it gets.

- State the rules it will otherwise break — a subagent does not inherit memory, and a system reminder may actively contradict them (trailers, in particular).
- Give it the facts you already established, with `file:line`. Make it verify what you are unsure of, and say which is which.
- **Ask for reasoning in the report, and say explicitly that it does not belong in the code.**
- Name what is out of scope, including other agents' files and anything uncommitted.
- Tell it which worktree to work in, and that cargo runs there and nowhere else.

---

# Part 2 — The code

## Commands

```sh
cargo run -p platform-win          # the terminal
cargo test                         # whole workspace
cargo test -p layout               # one crate
cargo test -p render beam_cursor   # one test by name substring
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

`cargo fmt --all` in write mode reformats every crate, uncommitted work included. Run `--check` first and format only your own files.

Diagnostics that need no window:

```sh
cargo run -p vt-core --bin spike -- "dir"        # pump a command, dump the grid as text
cargo run -p platform-win --bin keytest -- win32 # what actually reaches PSReadLine
cargo run -p render --example coverage           # which glyphs the font has
cargo run --release -p vt-core --example pane_cost -- 12   # the docs/02 §7 metric
```

**Never run `foot-win`** from an agent: it opens a window and steals focus. `cargo run -p platform-win` with no `--bin` is `foot-win`, so always name the binary. `keytest` and `spike` are headless and fine.

## Configuration

`%APPDATA%\foothy\foothy.ini`, in foot's ini format, with the 86 foot themes embedded in the binary and named rather than pathed.

Precedence, lowest to highest: `defaults < theme < state.ini < foothy.ini < FOOTWIN_*`.

**The app never writes `foothy.ini`.** Only `state.ini` is its own. A theme file may contribute colours and the light/dark variant, nothing else — it is text people copy from the internet.

Runtime tuning, no rebuild: `FOOTWIN_FONT`, `FOOTWIN_FONT_PX`, `FOOTWIN_PAD`, `FOOTWIN_FG`, `FOOTWIN_BG`, `FOOTWIN_ALPHA`, `FOOTWIN_BLINK`, `FOOTWIN_PASTE_WARN`.

## Architecture: the layer rule

This is the one thing to understand before changing anything.

| Crate | Layer | Owns |
|---|---|---|
| `vt-core` | 1 | VT parser, grid, scrollback, PTY, selection state |
| `render` | 2 | Glyph cache, grid → pixel buffer |
| `layout` | 2 | Pane tree: split, close, resize, directional navigation |
| `config` | 2 | ini parsing, the bundled themes, precedence merging, the theme filter |
| `input` | 2.5 | Key → bytes, win32-input-mode, paste safety |
| `platform-win` | 3 | winit, softbuffer, Win32 clipboard and window chrome |

**Layers 1, 2 and 2.5 must build and test without opening a window.** No `winit`, no `windows-sys`, no pixels beyond a rectangle they were handed. This is what keeps a second platform additive instead of a rewrite. Two thirds of the suite lives in layers 1-2.5, and the whole suite runs without a window. If a change needs the OS down there, the design is wrong — push the platform part up to `platform-win` and pass values down.

Check the rule in the **direct `[dependencies]` of each manifest**, not with `cargo tree`: `portable-pty` pulls `windows-sys` in transitively and that is fine.

`config`, `layout` and `input` have an empty `[dependencies]` on purpose. There the rule is a compile error, not a convention.

Three consequences worth knowing:

- `layout` uses **geometry, not tree order**, for directional navigation: most overlap on the perpendicular axis, ties broken by distance. Its tests hold the cases that break naive implementations.
- `vt-core` exposes a **read path** (`line_text`, `screen_text`, `is_selected`) on purpose. `docs/05` §3.1: a screen reader provider needs range-addressable reads, and retrofitting that is a refactor.
- **Policy belongs in the lowest layer that can hold it.** `config::parse_theme` parses and filters in one call, so the policy is written once in layer 2 instead of re-implemented per platform. `parse` is still public for user config files, so this narrows the mistake rather than making it impossible.

`docs/02` §3 lists ten performance rules taken from foot's source. Load-bearing today: bounded work per event-loop turn, and idle costs 0% CPU — the PTY reader wakes the loop through an `EventLoopProxy` instead of polling, and the blink timer only arms while focused.

## Gotchas that already cost time

- **ConPTY sends `ESC [ 6 n` on startup and blocks the child until answered.** Drop those replies and the terminal deadlocks with four bytes read. `Terminal::take_pty_replies` exists for this; `drain_events` deliberately leaves `PtyWrite` alone.
- **ConPTY never closes the master when the child exits**, so EOF never arrives. End of session is the child's exit status, and bytes are still in flight after it exits — drain until quiet or the tail is lost.
- **Modifier keys cannot be tested with synthetic input.** winit reads real keyboard state, which `PostMessage` does not update, so `Ctrl+…` needs a human. Non-modifier paths (middle-click paste, wheel, plain keys) can be driven with `PostMessage` plus `PrintWindow`, which also avoids stealing focus.
- **Writing Rust escape literals through a shell heredoc mangles them.** `\x1b` and `\r\n` arrive as raw control bytes. Use the Write/Edit tools for anything containing escapes.
- **A running `foot-win.exe` holds the binary.** Linking then fails with `os error 5`. Report it; never kill a process Emilio may own.
- **Grid indexing must account for `display_offset`**, or it is correct until the user scrolls. A wide character owns two cells and the second is a spacer, not a character.

## Reference docs

`docs/01` foot internals with file:line citations · `docs/02` the design doc, layers, performance rules, scope decisions · `docs/03` links · `docs/04` measurements, including this project's own baseline · `docs/05` behaviour and accessibility decisions, and the terminal-vs-shell split that explains why history and line editing are not ours to implement.
