---
description: Review a pull request for this terminal emulator and post the findings
argument-hint: "[PR number] (defaults to the PR for the current branch)"
allowed-tools: Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr comment:*), Bash(gh api:*), Bash(git log:*), Bash(git diff:*), Bash(cargo test:*), Bash(cargo clippy:*), Read, Grep, Glob
---

Target PR: `$1`

If that is empty, review the PR for the current branch — `gh pr view` and `gh pr diff` resolve it on their own when given no number.

Read the diff and the description first. Read surrounding code when the diff alone is not enough to judge a change: a hunk that looks fine in isolation is often wrong in context.

## What to look for, in priority order

**1. Layer discipline — the invariant this project lives by.**
`vt-core`, `render`, `layout` and `input` must build and test with no window. Flag any `winit`, `windows-sys`, `softbuffer` or other platform dependency reaching into them, and any platform concept (HWND, focus, blink phase, pixels beyond a passed-in rectangle) that leaked down instead of being passed as a value from `platform-win`. This is the check that keeps a second platform additive; treat a violation as blocking.

**2. Terminal correctness — where this domain bites.**
- Viewport vs buffer coordinates: anything indexing the grid must account for `display_offset`, or it breaks the moment the user scrolls.
- Wide characters: a CJK glyph owns two cells and the second is a spacer, not a character. Iterating cells without skipping `WIDE_CHAR_SPACER` corrupts text.
- PTY lifecycle: ConPTY answers queries through `PtyWrite` events and never sends EOF. Dropping replies deadlocks; treating EOF as end-of-session hangs; draining once after the child exits truncates output.
- Escape sequence handling split across reads — the parser is stateful for a reason.

**3. Performance rules** from `docs/02` §3, which exist because they are what makes foot fast:
- Work bounded per event-loop turn, so one noisy pane cannot starve others.
- Idle costs 0% CPU: event-driven wakeups, never polling, no timer armed when nothing is animating.
- No per-pane state that grows without a ceiling.
- Allocation or rasterisation on a hot path that should be cached.

**4. `unsafe` and Win32.**
Every `unsafe` block must be justified by the surrounding code. Check handle and memory ownership, that a Win32 call's return value is tested before use, that a resource acquired is released on every path, and that a struct passed by pointer has the size the API expects.

**5. Tests.**
New logic in layers 1–2.5 should be testable without a window, and usually is. Flag behaviour that could have had a headless test and did not get one, and tests that assert an implementation detail rather than the behaviour a user would notice.

**6. Project standards** (`AGENTS.md`).
Three of these are not visible in the diff, so look them up: run `gh pr view --json headRefName,commits` or `git log --oneline main..HEAD` before judging them.

- *Comments*: only for a rule that is important and not already obvious from the code — something whose removal would let someone reintroduce a bug. English, brief. Flag narration, commented-out code, and comments that restate the line below them.
- *Commit granularity*: one commit per file or per single focused aspect. Two or three files together only when the changes are minimal. Flag any commit touching a large set of files, and any single commit carrying several unrelated changes.
- *Commit messages*: one short English line, `type/short description` — slash, not colon. No body, no trailers. Types: `core`, `feat`, `fix`, `ui`, `docs`, `test`, `chore`, `refactor`, `perf`, `build`. Flag Spanish, multi-line messages, and missing or wrong types.
- *Branch name*: English, descriptive, self-explanatory. Flag Spanish or opaque names.

Report these in a short separate section of the summary, not as inline comments — they are about the history, not about a line.

## How to report

Post inline comments on the specific lines for concrete issues. Then post one top-level summary comment with the overall assessment.

Be direct and concise. Skip praise for the obvious. Say plainly when something is fine.

Rank findings by whether they can actually bite: a layer violation or a coordinate bug outranks a naming preference. If a finding is a guess, say so rather than stating it as fact — and prefer verifying it by reading the code or running `cargo test` over speculating.

If the diff is clean, say so in one line instead of inventing findings.
