# CLAUDE.md - Bot Instructions

## Project Structure

This project uses **two separate jj-git repos**:

1. **App repo** (`/` — project root): Contains the application source code.
2. **Bot session repo** (`/.claude/`): Contains Claude Code session data.

Both repos are managed with `jj` (Jujutsu), which coexists with git.

## Repo Paths (relative from project root)

- App repo: `.` (project root)
- Bot session repo: `.claude`
  (symlink from `~/.claude/projects/<path-to-project-root>/.claude`)

## Working Directory

Prefer staying in the project root. Use `-R` flags or absolute paths
to target other directories rather than `cd`. If `cd` seems necessary,
discuss with the user first — losing track of cwd causes subtle
command failures downstream.

## Memory

Do not use the bot's per-project memory directory
(`~/.claude/projects/<path>/memory/`). In a dual-repo setup with
CLAUDE.md it provides no capability CLAUDE.md doesn't already cover,
and it loses on discoverability:

- **CLAUDE.md** — at the repo root, a well-known location for bot
  instructions, committed, reviewable, visible to every collaborator
  (human or bot).
- **Memory directory** — hidden under the user's home, tied to one
  machine, invisible to anyone but the bot, never diffed or reviewed.

Easy for everyone to find beats convenient for the bot alone. Put
durable context in CLAUDE.md (or committed `notes/`) instead.

## Speculation marker

Durable text the bot writes — CLAUDE.md, `notes/`, commit bodies,
chores sections — should stick to observations and direct descriptions
of the code or data. If a mechanism, hypothesis, or causal claim
enters the text, prefix it with "The bot thinks ..." so a reader can
tell the measured from the inferred.

**Why:** unmarked speculation reads like evidence, and a future reader
(or the bot on a later session) can pick it up as a known fact when
it's not. Measured / inferred is a distinction worth keeping visible
in the written record.

**How to apply:** observations and factual descriptions need no
marker. Prefix with "The bot thinks ..." (or a close variant like
"The bot's guess is ...") when the claim is a mechanism ("X wins
because Y caches better"), a cause ("the drift was due to thermal
state"), a prediction ("this should scale linearly"), or any
reasoning not directly supported by the data on hand.

## Committing

Use `-R` (`--repository`) at the end to target the correct repo. Use
relative paths to reduce noise. Putting `-R` last keeps the verb/action
visible at the start of the command.

### App repo
```
jj commit -m \
"title" \
-m "body

ochid: /.claude/<changeID>" \
-R .
```

### Bot session repo
```
jj commit -m \
"title" \
-m "body

ochid: /<changeID>" \
-R .claude
```

## jj Basics

- `jj st -R .` / `jj st -R .claude` — show working copy status
- `jj log -R .` / `jj log -R .claude` — show commit log
- `jj commit -m "title" -m "body" -R <repo>` — finalize working copy into a commit
- `jj describe -m "title" -m "body" -R <repo>` — set description without committing
- `jj git push --bookmark <name> -R <repo>` — push a bookmark (no
  `--allow-new` flag; jj pushes new bookmarks without special flags)
- In jj, the working copy (@) is always a mutable commit being edited.
  `jj commit` finalizes it and creates a new empty working copy on top.
- The `.claude` repo always has uncommitted changes during an active
  session because session data updates continuously.

## Commit Message Style

Use [Conventional Commits](https://www.conventionalcommits.org/) with
a version suffix:

```
<type>: <short description> (<version>)
```

- **Title**: target ~50 chars, short summary of *what* changed.
  Include the version. Common types: `feat`, `fix`, `refactor`,
  `test`, `docs`, `chore`.
- **App-repo body**: short intro paragraph (1–3 sentences), then a
  terse bullet list. Each bullet corresponds one-to-one with the
  edits structure already documented in `notes/chores-*.md` for
  this step — just the file and a one-line gist (e.g.
  `README.md: new Overview intro`). Do *not* restate the detail
  that lives in chores; the commit body is a scan-able index, not
  a duplicate. The chores section is the source of truth.
- **Session-repo body**: terse intro + a few session-activity
  bullets. Doesn't need to mirror chores since it describes
  in-session work, not code changes.
- Examples:
  - `feat: add fix-ochid subcommand (0.22.0)`
  - `fix: fix-ochid prefix bug (0.22.1)`
  - `refactor: deduplicate common CLI flags (0.21.1)`

## Pre-commit Requirements

### User approval

Never execute commit, squash, push, or finalize commands without the
user's explicit approval. Present changes for review first; only run
them after the user confirms. This applies to late changes too —
pause for review before squashing into an existing commit.

### Review before proposing the commit block

After finishing a unit of work, **summarize what changed and stop
there**. Do not pre-emptively lay out the Checkpoint-1 commit
commands. Wait for the user to signal review is complete before
proposing the commit block. Changes during review are the norm,
not the exception; proposing commit text too early creates noise
and signals that I consider the work done when it usually isn't.

This applies per-step in a multi-step flow too — each step gets a
review pause before its commit block appears.

Signals that review is complete include explicit approval ("let's
commit", "looks good, commit it") **and any directive to start the
next step** ("do step 4", "next", "go N+1"). In that case the
previous step must be committed first — always commit the current
step before starting the next; don't ask.

### Notes references

Multiple references must be separated: `[2],[3]` not `[2,3]` or `[2][3]`.
See [Todo format](notes/README.md#todo-format) for details.

### Versioning

Every plan must start with a version bump. Choose the approach based
on scope:

- **Single-step** (recommended for mechanical/focused changes): bump
  directly to `X.Y.Z`, implement in one commit. Simpler history.
- **Multi-step** (for exploratory/large changes): bump to `X.Y.Z-0`,
  implement across multiple commits incrementing the numeric
  suffix. The final commit drops the suffix.

The plan should recommend one approach and get user approval before
starting.

For multi-step:

1. Bump version to `X.Y.Z-0` with the plan and commit as a chore
   marker.
2. Implement in one or more `X.Y.Z-N` commits (increment N as
   needed).
3. Final commit bumps to `X.Y.Z` (no suffix), updates
   `notes/todo.md` and `notes/chores-*.md` — this is the "done"
   marker.

**Why numeric suffixes (`-0`, `-1`, …) rather than `-devN`:**
semver pre-release identifiers may consist of a single numeric
component, and they compare numerically per spec. So
`X.Y.Z-1 < X.Y.Z-2 < … < X.Y.Z` correctly orders the dev ladder
below the done marker. Cargo accepts this form. The `-dev` prefix
adds no information the git log doesn't already convey and
doubles typing per commit.

The final release commit (no suffix) signals completion rather than
amending prior commits. This keeps history readable and makes it easy
to see which commits were exploratory vs final.

### Chores section headers

Chores section headers use trailing version format:

```
## Description (X.Y.Z)
```

Example: `` ## Add `fn claude-symlink` (0.27.0) ``

### Pre-commit checklist

Before proposing a commit, run all of the following and fix any issues:

1. `cargo fmt`
2. `cargo clippy`
3. `cargo test`
4. `cargo install --path .` (if applicable)
5. Retest after install
6. Update `notes/todo.md` — add to `## Done` if completing a task
7. Update `notes/chores-*.md` — add a subsection describing the change
8. Update `notes/README.md` — if functionality changed (new flags,
   new subcommands, changed behavior)

## Code Conventions

### Doc comments on every file, function, and method

Every `.rs` file must begin with a `//!` module docstring. Every
function and method must have a `///` doc comment. Keep them brief —
one sentence of purpose is often enough; the discipline is that the
comment exists, not that it be long.

This is a deliberate override of the generic "write no comments"
default that applies to inline `//` comments. Doc comments on the
module / item surface are expected; inline explanatory comments
inside function bodies remain discouraged unless they capture a
non-obvious WHY.

### `// OK: …` comments on `unwrap*` calls (Rust)

Non-test code that calls `.unwrap()`, `.unwrap_or(…)`,
`.unwrap_or_default()`, or `.unwrap_or_else(…)` must have a trailing
`// OK: …` comment that justifies why the call is acceptable.

- `// OK: <specific reason>` — document the real precondition,
  invariant, or domain reason. Preferred whenever the reason isn't
  self-evident.
- `// OK: obvious` — the default is self-evident from context (e.g.
  `desc.lines().next().unwrap_or("")` — empty desc → empty title).

Bare `// OK` is not used (reads like a truncated comment).
Abbreviations (e.g. `SE`) are not used because they require a decoder
ring for readers seeing the code out of context.

For provably-unreachable `.unwrap()` calls, also prefix with
`#[allow(clippy::unwrap_used)]` so the site stays silent if we enable
the project-wide `clippy::unwrap_used` lint later.

```rust
let max = stderr_level.unwrap_or(LevelFilter::Info); // OK: default verbosity when -v/-vv absent
let first_line = desc.lines().next().unwrap_or("");  // OK: obvious

match matches.len() {
    1 => {
        #[allow(clippy::unwrap_used)]
        // OK: `1 =>` arm guarantees matches.len() == 1
        Ok(TitleMatch::One(matches.into_iter().next().unwrap()))
    }
    // ...
}
```

Tests (`#[cfg(test)]`) are exempt — panicking on setup failure is the
correct test behavior.

### Ask for clarification on ambiguous input

When user input is ambiguous or missing necessary detail, stop and
ask a specific question. Do not proceed on a guess and hope the
result lands right — a clarifying question costs a few seconds;
redoing misaligned work costs much more.

### Recognize when stuck

If a simple task has eaten 5+ minutes of thinking or back-and-forth
without progress, stop. Summarize what's blocking — unclear
requirements, unfamiliar API surface, conflicting signals — and ask.
Continued flailing produces worse outcomes than a direct "I'm stuck
on X."

## ochid Trailers

Every commit body must include an `ochid:` trailer pointing to the
counterpart commit in the other repo. The value is a workspace-root-relative
path followed by the changeID:

- App repo commits point to `.claude`: `ochid: /.claude/<changeID>`
- Bot session commits point to app repo: `ochid: /<changeID>`

Use `vc-x1 chid -R .,.claude -L` to get both changeIDs (first line
is app repo, second is `.claude`).

## Commit-Push-Finalize Flow

Two-checkpoint flow with explicit user approval at each stage.

**Run this flow after every step** — not only at session end.
Single-step and multi-step changes are of equal importance: a
single-step change is one flow; a multi-step change is one flow per
`X.Y.Z-N` commit plus one for the final release commit. Each step
gets its own commits, its own push, and its own finalize — so dev
markers land on the remote and in the `.claude` history as they
happen rather than being batched until the end.

### Checkpoint 1: Commit

Prepare both commit commands and **present them for approval**. Use
the **same title** for both commits so they're easy to correlate.
The body can differ: the app repo body should summarize code
changes; the bot session repo body should note what was done in the
session.

On approval, execute the commits and set bookmarks:

```
jj commit -m "shared title" -m "app body" -R .
jj commit -m "shared title" -m "session body" -R .claude
jj bookmark set <bookmark> -r @- -R .
jj bookmark set <bookmark> -r @- -R .claude
```

### Checkpoint 2: Push and finalize

After commits succeed, **ask the user to approve push and finalize**.
On approval, push the app repo and finalize the bot session in a
single operation. Say any final words (e.g. "next is ...") **before**
executing — nothing should be output after finalize.

```
jj git push --bookmark <bookmark> -R . && vc-x1 finalize --repo .claude --squash <SOURCE,TARGET> --push <bookmark> --delay 10 --detach --log /tmp/vc-x1-finalize.log
```

Replace `<bookmark>` with the active bookmark (e.g. `main`,
`dev-0.14.0`). Do **not** push `.claude` separately — `finalize`
handles that push after squashing trailing writes.

### After finalize: stop and wait

After `vc-x1 finalize` is launched — **whether mid-session per-step
or at session end** — you **MUST NEVER** proceed to a next step, edit
files, run tools, or emit any text (prose, recaps, acknowledgements),
until the user explicitly directs you to continue. Treat finalize as
a hard stop for the whole turn. Any final words (e.g. "next is ...")
must be said in the approval prompt *before* executing finalize; the
finalize `Bash` call is the last thing in the turn and nothing
follows it.

This holds even when the next step seems obvious (e.g. "next is
N+1" or "now I should bump the version and commit the release").
Wait. The user controls cadence — every push+finalize is a checkpoint
they may want to inspect, think about, hand off, or take a break at.
Auto-proceeding bypasses that checkpoint and produces unwanted writes
between finalize and the next explicit instruction.

Exceptions to this rule may emerge later but are not authorized at
this stage. Until told otherwise, treat as absolute.

### Late changes after push

If changes are made to the app repo after it has been pushed (e.g.
updating CLAUDE.md or memory), the commit is now immutable. Use
`--ignore-immutable` to squash the changes in, then re-push:

```
jj squash --ignore-immutable -R .
jj bookmark set <bookmark> -r @- -R .
jj git push --bookmark <bookmark> -R .
```

### Finalize the .claude repo

The **very last action** in a session is to finalize the `.claude`
repo. `--squash @,@-` squashes the working copy into the session
commit. The delay gives a safety margin against any pending writes.
Always use a short relative path for `--repo`.

**Nothing should happen after finalize** — no memory writes, no tool
calls, no additional output. If any work is done after finalize, run
finalize again so the trailing writes are captured.

```
vc-x1 finalize --repo .claude --squash --push <bookmark> --delay 10 --detach --log /tmp/vc-x1-finalize.log
```

Do **not** echo or restate the finalize output — the Bash tool
already displays it. Any trailing text output creates writes that
miss the finalize squash window.
