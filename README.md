# actor-x1

An experiment in creating an actor model in rust
see [notes/design.md](notes/design.md) for details.

This is the main repo of a dual-repo convention for using
a bot to help in the development of a coding project. The goal
is that this main repo contains the "what", while the partner
bot repo contains "why" and "how". The key to the convention
is each change is cross-referenced to the other. Thus there
is a coherent story of the development of the project across time.

The beginnings of that tool is [vc-x1](https://github.com/winksaville/vc-x1)
which currently does achieve this goal, but is being used as a
first test bed.

## Cloning

The Bot session repo is setup as a git submodule which
makes it easier to clone both repos. The easiest is to
clone both when doing the initial clone:
```
git clone --recurisve git@github.com:winksaville/vc-template-x1
```

If you forget to use --recurisze you need to do two additional
steps "init" and "update":
```
git clone git@github.com:winksaville/vc-template-x1
git submodule init
git submodule update
```

And these can be combined using 'update --init' if you like
```
git clone git@github.com:winksaville/vc-template-x1
git submodule update --init
```

## Usage

Stage 1 (the PoC landed at 0.1.0) provides two binaries:

- **`goal1`** — two actors on one thread, ping-ponging a
  message through a shared `VecDeque`.
- **`goal2`** — two actors on two threads, ping-ponging over
  `std::sync::mpsc` channels.

Install with `cargo install --path crates/actor-x1`; both share
a common CLI shape:

```
$ goal1 0.5 --warmup 0 --inner 1000 --pin 0
actor-x1 0.2.0
goal1: 102087000 messages in 0.500s (204.174 M msg/s, inner=1000)
  pinning: main → core 0
  apparatus: framing=21 tk (5.54 ns); per-event at inner=1000 = 0 tk (0.00 ns)

  tprobe: goal1.dispatch [count=102,087]
              first     last    range        count mean
    min-p1        0 ns     0 ns     0 ns         0    0 ns
    p1-p10        0 ns     0 ns     0 ns         0    0 ns
    p10-p20       4 ns     4 ns     0 ns    32,684    4 ns
    p20-p30       0 ns     0 ns     0 ns         0    0 ns
    p30-p40       0 ns     0 ns     0 ns         0    0 ns
    p40-p50       5 ns     5 ns     0 ns    30,128    5 ns
    p50-p60       0 ns     0 ns     0 ns         0    0 ns
    p60-p70       0 ns     0 ns     0 ns         0    0 ns
    p70-p80       5 ns     5 ns     0 ns    34,218    5 ns
    p80-p90       0 ns     0 ns     0 ns         0    0 ns
    p90-p99       5 ns     7 ns     2 ns     3,971    6 ns
    p99-max       7 ns    34 ns    27 ns     1,086    9 ns
    mean                                              5 ns
    stdev                                             1 ns
    mean min-p99                                      5 ns
    stdev min-p99                                     0 ns
```

### Flags

- **`<duration_s>`** — positional, required; measurement window
  in seconds.
- **`--warmup <SECS>`** — default 10.0; run the same dispatch
  loop with the probe active for this long, then clear the probe
  at the boundary so only steady-state data reaches the report.
  Set to 0 for quick iteration.
- **`--inner <N>`** *(goal1 only)* — default 1, ≥1; dispatches
  per probe scope. Larger values amortize the probe's two-`rdtsc`
  apparatus cost and give the histogram meaningful dynamic range.
  goal2 fixes `inner=1` because its per-channel queue holds at
  most one message.
- **`-t` / `--ticks`** — display probe values as raw ticks
  instead of nanoseconds.
- **`--raw`** — skip apparatus-overhead calibration; show the raw
  per-event cost including framing contamination.
- **`--pin <CORES>`** — pin threads to logical CPUs. goal1 uses
  the first core in the list; goal2 pins actor `i` to
  `cores[i % len]`. Accepts comma lists and ranges: `--pin 5`,
  `--pin 0,1`, `--pin 0-3`, `--pin 0,0` (oversubscribe).

### Typical invocations

```
# Short iterative smoke, no warmup:
goal1 0.1 --warmup 0 --inner 1000

# Full measurement with pinning:
goal1 2 --inner 1000 --pin 0

# Goal2 with actors on separate cores (same complex):
goal2 1 --pin 0,1
```

## Reading the band table

The `tprobe` block is a 12-row percentile histogram with four
summary rows. Every row renders every run, even when a band
holds no data — a zero row is distinguishable from a missing row.

### Header

```
tprobe: <name> [count=N]
```

`<name>` identifies the probe (`goal1.dispatch`,
`goal2.dispatch.actor0`, …). `count=N` is the number of **scopes**
drained into the histogram, not total dispatches — at `--inner
1000` one scope covers 1,000 dispatches, so `count` here is
`(total_messages / inner)`. Total dispatch count is in the summary
line above.

### Rows

Twelve percentile bands in order: `min-p1`, `p1-p10`, …, `p90-p99`,
`p99-max`. Each row covers the samples whose rank in the sorted
distribution falls within that percentile range.

### Columns

- **first** — lowest value observed in the band.
- **last** — highest value observed in the band.
- **range** — `last - first + 1`; width of the band's value range.
- **count** — number of samples in the band.
- **mean** — average value within the band.

### Summary rows

- **mean / stdev** — whole-histogram statistics, including the
  `p99-max` tail.
- **mean min-p99 / stdev min-p99** — same statistics excluding
  `p99-max`. Useful for A/B-ing code changes because the tail can
  be dominated by rare outliers (OS preemption, lock contention,
  interrupts) that drown signal in the body of the distribution.

### Why a band can be all zeros

The renderer walks `hdrhistogram` buckets and assigns each bucket
to one band by its cumulative mid-rank. A tight distribution has
most samples in a handful of buckets, and each bucket lands
entirely in one band — so neighboring bands show zeros even
though they'd nominally hold ~10% of the data. It's a display
artifact, not missing data: the samples are visible in the
adjacent non-zero band. A `stdev min-p99` near 0 alongside a
couple of dense bands is a strong signal that the underlying
distribution is spike-heavy.

### The `apparatus:` line

```
apparatus: framing=21 tk (5.54 ns); per-event at inner=1000 = 0 tk (0.00 ns)
```

Reports the fitted overhead model. `framing` is the hardware
floor cost of the probe's two-`rdtsc` pair, measured by a
two-point fit on empty `black_box(1)` scopes. `per-event at
inner=N` is what the report subtracts from each stored value —
`framing / N`, which amortizes toward zero as `inner` grows.
Under `--raw` this line reads `raw (no overhead subtraction)`
and calibration is skipped.

### The `pinning:` line

```
pinning: actor0→core0, actor1→core4
```

Shows what `--pin` produced for each workload thread;
`none (unpinned)` means the OS is free to schedule threads
anywhere.

## The bot's understanding

A handful of recurring patterns in the output, with the bot's
best read of why the numbers look the way they do. Specific
numbers below are from an AMD Ryzen 3900X at ~3.7 GHz — compare
your own numbers to your own baseline, not to these.

### goal1 `--inner 1` shows ~9 ns; `--inner 1000` shows ~5 ns

The probe's two-`rdtsc` framing has a real cost (~4 ns on modern
x86) that contaminates every stored scope delta. At `inner=1`
the full framing lands on every sample, pushing the reported
per-event cost to ~9 ns; at `inner=1000` framing is amortized
(~0.004 ns per event), so the stored value converges on the
actual `handle_message` cost of ~5 ns. `--raw` makes this
concrete — `--raw --inner 1` reads ~9 ns, `--raw --inner 1000`
reads ~5 ns. Default overhead calibration subtracts the framing,
so adjusted `--inner 1` also lands at ~5 ns, matching
`--inner 1000`.

### goal2 is ~900× slower than goal1 `--inner 1000`

goal1 `--inner 1000`: ~5 ns per event (~200 M msg/s).
goal2 `--inner 1`: ~1–1.5 µs per event (~0.25 M msg/s).
The bot thinks the gap is dominated by:

- **`mpsc`'s `Mutex` + condvar wake** on every send and recv —
  even a fast bounded mpsc pays this on every cross-thread
  hand-off.
- **Two context switches per round-trip** — one per channel
  hand-off, so a ping-pong cycle of (A send → B recv → B send →
  A recv) crosses the scheduler four times.
- **Cache-line bouncing** — the shared channel metadata and the
  message migrate between the two actors' L1 caches each round.

An unbounded lock-free mpsc would close some of this; the
context-switch floor is fundamental to any "block, wake, run"
channel pattern.

### `--pin` tightens `stdev` but can raise the mean

Pinning eliminates OS thread-migration noise — a thread landing
on a different core than the previous message's cache
warmed — so `stdev min-p99` drops (on this box: 246 ns → 80 ns,
about 3× tighter). But pinning also *forces* placement, which
can be worse than what the OS would have chosen. On this 3900X
(12 cores split across 4 CCX complexes of 3 cores each),
`--pin 0,4` puts the two actors on different CCX complexes, so
every ping-pong round forces a cross-complex cache coherence
transfer — the mean rises from ~630 ns to ~1,500 ns. `--pin 0,1`
(same CCX) keeps the mean near unpinned while still tightening
the stdev.

This isn't a flaw; it's the intended trade. Pinning gives you
control over what you measure — pick cores that match the
scenario you care about (best-case same-complex, worst-case
cross-socket, etc.).

### The `p99-max` tail occasionally spikes to tens of µs or ms

Almost always an OS interrupt, scheduler preemption, or a rare
lock contention sneaking into a probe scope. The probe sees real
events the system experiences, so the measurement is honest —
they just aren't what you typically want to compare across
runs. `mean min-p99` excludes this tail and is usually the
right comparator for A/B work.

### Numbers travel with the hardware

- **Framing** is ~20–50 ticks regardless of clock speed
  (pipeline cost in cycles, not time).
- **goal1 per-event cost at large `inner`** scales with clock
  frequency.
- **goal2 latency** depends heavily on CPU topology (single
  socket vs NUMA), cache architecture, and scheduler tuning.

If the numbers on your box differ from those above, that's
expected; consistency run-to-run and sensitivity to
`--pin` / `--inner` / `--raw` are the invariants worth trusting.

## jj Tips for Git Users

See [Steve Klabnik](https://github.com/steveklabnik)
[Jujutsu-tutorial](https://steveklabnik.github.io/jujutsu-tutorial)
and [jj docs](https://docs.jj-vcs.dev/latest/).

### Initial Commit for a repo

Create create directory add files.

Minimal commands to push 

```
jj git init .
jj describe
jj git remote add origin git@github.com:winksaville/vc-template-x1
jj bookmark create main -r @
jj bookmark track main --remote=origin
jj git push
```

### Push a change to main

Assuming that this is to be push to main you
set the bookmark to the appropriate commit and
then just push:

```
jj bookmark set main -r @
jj git push
```

Complete example:
```
wink@3900x 26-03-13T17:26:21.177Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1a79f803025f75fb557a7b6f9d29e3dbee6a1724))
$ vi README.md 
wink@3900x 26-03-13T17:28:08.833Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1a79f803025f75fb557a7b6f9d29e3dbee6a1724))
$ jj log
@  vnsyoswv wink@saville.com 2026-03-13 10:28:15 main* 3ac24f49
│  feat: Update README.md
◆  vuwzvmwm wink@saville.com 2026-03-13 09:38:22 main@origin 1a79f803
│  feat: Initial commit for the vibe coding main repo
~
wink@3900x 26-03-13T17:28:15.704Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1a79f803025f75fb557a7b6f9d29e3dbee6a1724))
$ jj git push
Changes to push to origin:
  Move forward bookmark main from 1a79f803025f to 3ac24f49321b
git: Enumerating objects: 5, done.
git: Counting objects: 100% (5/5), done.
git: Delta compression using up to 24 threads
git: Compressing objects: 100% (3/3), done.
git: Writing objects: 100% (3/3), 790 bytes | 790.00 KiB/s, done.
git: Total 3 (delta 2), reused 0 (delta 0), pack-reused 0 (from 0)
remote: Resolving deltas: 100% (2/2), completed with 2 local objects.
Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
Working copy  (@) now at: kywoutls c26d415e (empty) (no description set)
Parent commit (@-)      : vnsyoswv 3ac24f49 main | feat: Update README.md
wink@3900x 26-03-13T17:28:33.741Z:~/data/prgs/rust/vc-template-x1 ((main))
```

### Example of modifying an existing commit and "force" push

Tweak a commit and push it using `jj edit` then "force" push:

Minimum steps changing xx but it could be any commit on main
or other bookmark/branch the last step repositions @ so @- is main:

```
jj edit -r xxx --ignore-immutable
<Modify the commit such as, `jj describe or `vi README.md`>
jj git push --bookmark main
jj new main
```

A complete example, the `jj log` commands are to just give
a little more visibility. The thing I'm changing is the conventaional
commit type for of vnsyoswv is "feat" is should be "docs":
```
wink@3900x 26-03-13T17:32:17.819Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1a79f803025f75fb557a7b6f9d29e3dbee6a1724))
$ jj log -r ::@
@  uxuqmtov wink@saville.com 2026-03-13 10:53:15 d4205bc4
│  (empty) (no description set)
◆  plkoouwq wink@saville.com 2026-03-13 10:50:54 main e76950c0
│  docs: Update README.md with force push example
◆  vnsyoswv wink@saville.com 2026-03-13 10:32:32 525123b1
│  feat: Update README.md
◆  vuwzvmwm wink@saville.com 2026-03-13 09:38:22 1a79f803
│  feat: Initial commit for the vibe coding main repo
◆  zzzzzzzz root() 00000000
wink@3900x 26-03-13T17:57:13.692Z:~/data/prgs/rust/vc-template-x1 ((main))
$ jj edit -r vn --ignore-immutable 
Working copy  (@) now at: vnsyoswv 525123b1 feat: Update README.md
Parent commit (@-)      : vuwzvmwm 1a79f803 feat: Initial commit for the vibe coding main repo
Added 0 files, modified 1 files, removed 0 files
wink@3900x 26-03-13T17:57:27.856Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1a79f803025f75fb557a7b6f9d29e3dbee6a1724))
Rebased 1 descendant commits
Working copy  (@) now at: vnsyoswv 1b6ed25c docs: Update README.md
Parent commit (@-)      : vuwzvmwm 1a79f803 feat: Initial commit for the vibe coding main repo
wink@3900x 26-03-13T17:58:34.975Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1a79f803025f75fb557a7b6f9d29e3dbee6a1724))
$ jj log
○  plkoouwq wink@saville.com 2026-03-13 10:58:34 main* bc66029d
│  docs: Update README.md with force push example
@  vnsyoswv wink@saville.com 2026-03-13 10:57:53 1b6ed25c
│  docs: Update README.md
│ ◆  plkoouwq/1 wink@saville.com 2026-03-13 10:50:54 main@origin e76950c0 (hidden)
│ │  docs: Update README.md with force push example
│ ~  (elided revisions)
├─╯
◆  vuwzvmwm wink@saville.com 2026-03-13 09:38:22 1a79f803
│  feat: Initial commit for the vibe coding main repo
~
wink@3900x 26-03-13T18:15:39.052Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1a79f803025f75fb557a7b6f9d29e3dbee6a1724))
$ jj log -r ::main
○  plkoouwq wink@saville.com 2026-03-13 10:58:34 main* bc66029d
│  docs: Update README.md with force push example
@  vnsyoswv wink@saville.com 2026-03-13 10:57:53 1b6ed25c
│  docs: Update README.md
◆  vuwzvmwm wink@saville.com 2026-03-13 09:38:22 1a79f803
│  feat: Initial commit for the vibe coding main repo
◆  zzzzzzzz root() 00000000
wink@3900x 26-03-13T18:17:20.926Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1a79f803025f75fb557a7b6f9d29e3dbee6a1724))
$ jj git push --bookmark main
Changes to push to origin:
  Move sideways bookmark main from e76950c0c352 to bc66029d050c
git: Enumerating objects: 8, done.
git: Counting objects: 100% (8/8), done.
git: Delta compression using up to 24 threads
git: Compressing objects: 100% (6/6), done.
git: Writing objects: 100% (6/6), 3.50 KiB | 3.50 MiB/s, done.
git: Total 6 (delta 3), reused 0 (delta 0), pack-reused 0 (from 0)
remote: Resolving deltas: 100% (3/3), completed with 1 local object.
Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
Working copy  (@) now at: srxnytso 22165d77 (empty) (no description set)
Parent commit (@-)      : vnsyoswv 1b6ed25c docs: Update README.md
wink@3900x 26-03-13T18:19:07.922Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1b6ed25cf716ba3686bed15085f0463590a6200c))
$ 
wink@3900x 26-03-13T18:22:21.776Z:~/data/prgs/rust/vc-template-x1 ((jj/keep/1b6ed25cf716ba3686bed15085f0463590a6200c))
$ jj new main
Working copy  (@) now at: vytkmroy 8df04518 (empty) (no description set)
Parent commit (@-)      : plkoouwq bc66029d main | docs: Update README.md with force push example
Added 0 files, modified 1 files, removed 0 files
wink@3900x 26-03-13T18:25:23.243Z:~/data/prgs/rust/vc-template-x1 ((main))
$ jj log -r ::@
@  vytkmroy wink@saville.com 2026-03-13 11:25:23 8df04518
│  (empty) (no description set)
◆  plkoouwq wink@saville.com 2026-03-13 10:58:34 main bc66029d
│  docs: Update README.md with force push example
◆  vnsyoswv wink@saville.com 2026-03-13 10:57:53 1b6ed25c
│  docs: Update README.md
◆  vuwzvmwm wink@saville.com 2026-03-13 09:38:22 1a79f803
│  feat: Initial commit for the vibe coding main repo
◆  zzzzzzzz root() 00000000
wink@3900x 26-03-13T18:25:46.005Z:~/data/prgs/rust/vc-template-x1 ((main))
$
```

### Why `jj log` shows fewer commits than `gitk`

If you're coming from git, jj's log output can be surprising compared to
tools like `gitk --all`.

jj tracks *changes* (identified by change IDs), not individual git commits.
When you rewrite a change (`jj describe`, `jj rebase`, `jj squash`, etc.),
jj creates a new git commit and keeps the old one under `refs/jj/keep/*` as
undo history. `gitk --all` sees all of these obsolete commits; `jj log` only
shows the current version of each change.

### Useful commands

| Command | Description |
|---------|-------------|
| `jj log` | Show recent visible commits (default revset) |
| `jj log -r ::@` | Show **all** ancestors of the working copy |
| `jj log -r 'all()'` | Show all non-hidden commits (needed if you have multiple heads/branches) |
| `jj st | Show the status of the Working and Parent commits |
| `jj st -r <chid> | Status of the commit, <chid> such as `@`, `@-`, `xyz` |
| `jj show | Show the Working commit, -r @ |
| `jj show -r <chid> | Show the commit, <chid> such as `@`, `@-`, `xyz` |
| `jj evolog -r <chid>` | Show the evolution history of a single change |
| `jj op log` | Show operation history (each rewrite operation) |


In a single-branch workflow, `jj log -r ::@` and `jj log -r 'all()'` give
the same result. Use `all()` when you have multiple branches or heads.

## Cross-repo Linking with Git Trailers

Commits in each repo use [git trailers](https://git-scm.com/docs/git-interpret-trailers)
to cross-reference their counterpart in the other repo. The `ochid`
(Other Change ID) trailer contains a workspace-root-relative path
and jj changeID:

```
ochid: /.claude/xvzvruqo   # points to a .claude repo change
ochid: /wtpmottv            # points to an app repo change
```

Paths always start with `/` (the workspace root, i.e. vc-x1).
Each repo has a `.vc-config.toml` that identifies its location
within the workspace, so tools can resolve these paths locally.

For full details see:
- [Git trailer convention](./notes/chores-01.md#git-trailer-convention)
  — [ochid (Other Change ID)](./notes/chores-01.md#ochid-other-change-id)
  — [ChangeID path syntax](./notes/chores-01.md#changeid-path-syntax)
  — [.vc-config.toml](./notes/chores-01.md#vc-configtoml)

## Contributing

Bot-following workflow, commit conventions, and code style are
canonical in [CLAUDE.md](CLAUDE.md):

- [Versioning during development](CLAUDE.md#versioning) — `-N`
  pre-release suffix convention (single-step vs multi-step).
- [Commit message style](CLAUDE.md#commit-message-style).
- [Commit-Push-Finalize Flow](CLAUDE.md#commit-push-finalize-flow) —
  two-checkpoint per-step discipline.
- [Code Conventions](CLAUDE.md#code-conventions) — doc comments on
  every file / fn / method, `// OK: …` on `unwrap*` calls,
  ask-on-ambiguity, stuck detection.
- [Pre-commit checklist](CLAUDE.md#pre-commit-checklist).

Task tracking and release details live under [notes/](notes/):
near-term tasks in [notes/todo.md](notes/todo.md), per-release
details in `notes/chores-*.md`, and notes-specific formatting
rules in [notes/README.md](notes/README.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.

[1]: https://github.com/karpathy/autoresearch
