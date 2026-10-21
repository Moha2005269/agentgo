# AgentGo 0.1.6 benchmarks

0.1.6 against 0.1.5, with GoMini and without, on the same tasks, the same
machine (Windows 11, Ryzen, RX 7800 XT) and the same model, 2026-10-21 and 22.
The work started from two benchmarks of 0.1.5: the SPA set
([agentgo-spa.md](agentgo-spa.md)), where AgentGo clicked the wrong row after
a re-render, and 154 runs of AgentGo + GoMini, where the planner never
delegated a step to GoMini. Every fix below is checked by a test that drives
the browser with no model (`benchmarks/v016/smoke.py`, `stress.py`), and the
whole is measured by an agent.

## Method

- **Agent:** gpt-6-luna (medium reasoning) through Factory's `droid exec`, a
  fresh session for every task, only the browser's MCP tools (text tools: no
  screenshots, no page script) and the server's own instructions; the prompt is
  the page address and the task. The page decides success; tokens, turns and
  time are droid's own usage record.
- **Tasks:** 38: the 24 MiniWoB++ tasks of the 0.1.5 report (seeds 41 and 51)
  and the 14 SPA tasks (11 local self-grading apps, 3 live pages).
- **Setups:** AgentGo 0.1.5 with GoMini v5 and without GoMini; AgentGo 0.1.6
  (build of commit `0.1.6: browser_wait gone: words are a text`, one fix
  before the release build) with GoMini v6 and without.
- **At the same time.** The model API's latency drifts by tens of percent
  over an evening, so the four setups run together, one worker each, and the
  whole set ran twice (76 runs per setup). Measured one after another, 0.1.6
  looked slower while its browser calls were faster (a click took about 490
  ms against 660 ms on 0.1.5, timed with no model).
- **One run per task and seed, twice.** Single tasks still move: one solved
  click-pie cost 1.5M tokens. Results are given with and without click-pie.

## Results (76 runs per setup)

| Setup | Solved | Wrong actions | Turns | Input tokens per task | Per solved task | Median s | Mean s |
|---|---:|---:|---:|---:|---:|---:|---:|
| 0.1.5 + GoMini v5 | 66/76 (86.8%) | 6 | 10.2 | 133,759 | 154,026 | 38.9 | 57.0 |
| **0.1.6 + GoMini v6** | **72/76 (94.7%)** | **0** | 10.0 | 144,423 | 152,447 | 38.4 | 52.0 |
| 0.1.5 alone | 70/76 (92.1%) | 2 | 10.9 | 149,046 | 161,822 | 41.0 | 53.0 |
| **0.1.6 alone** | 70/76 (92.1%) | **1** | **9.4** | **124,745** | **135,437** | **37.1** | **49.4** |

Without click-pie (74 runs per setup):

| Setup | Solved | Turns | Input tokens per task | Per solved task | Mean s |
|---|---:|---:|---:|---:|---:|
| 0.1.5 + GoMini v5 | 89.2% | 10.2 | 133,381 | 149,548 | 51.5 |
| **0.1.6 + GoMini v6** | **95.9%** | **9.3** | **122,664 (-8%)** | **127,847 (-15%)** | **48.4 (-6%)** |
| 0.1.5 alone | 94.6% | 10.4 | 136,381 | 144,174 | 50.7 |
| **0.1.6 alone** | 94.6% | **8.9 (-14%)** | **113,213 (-17%)** | **119,683 (-17%)** | **47.5 (-6%)** |

By suite:

| Setup | MiniWoB++ solved | MiniWoB++ tokens/task | SPA solved | SPA tokens/task | SPA wrong actions |
|---|---:|---:|---:|---:|---:|
| 0.1.5 + GoMini v5 | 43/48 | 94,026 | 23/28 | 201,873 | 4 |
| 0.1.6 + GoMini v6 | 45/48 | 134,050* | 27/28 | 162,206 (-20%) | 0 |
| 0.1.5 alone | 44/48 | 118,551 | 26/28 | 201,324 | 2 |
| 0.1.6 alone | 44/48 | 104,874 (-12%) | 26/28 | 158,808 (-21%) | 1 |

\* includes the solved click-pie (1.5M tokens).

### Task by task

Solved of 2, mean time, mean input tokens.

| Task | 0.1.5 + GoMini v5 | 0.1.5 alone | 0.1.6 + GoMini v6 | 0.1.6 alone |
|---|---|---|---|---|
| virtual-list | 2/2 · 126 s · 400k | 2/2 · 86 s · 249k | 2/2 · 40 s · 78k | 2/2 · 35 s · 59k |
| unkeyed-inbox | **0/2** · 64 s · 224k | **0/2** · 73 s · 302k | 2/2 · 28 s · 46k | 2/2 · 24 s · 46k |
| email-inbox-forward-nl | 2/2 · 122 s · 393k | 2/2 · 115 s · 416k | 2/2 · 102 s · 397k | 2/2 · 49 s · 129k |
| infinite-feed | 2/2 · 89 s · 281k | 2/2 · 81 s · 262k | 2/2 · 59 s · 182k | 2/2 · 62 s · 170k |
| optimistic-star | 2/2 · 101 s · 338k | 2/2 · 109 s · 397k | 2/2 · 74 s · 182k | 2/2 · 91 s · 302k |
| skeleton-orders | 2/2 · 40 s · 95k | 2/2 · 42 s · 82k | 2/2 · 34 s · 65k | 2/2 · 35 s · 65k |
| keyed-leaderboard | **0/2** · 37 s · 84k | 2/2 · 30 s · 46k | 2/2 · 29 s · 46k | 1/2 · 31 s · 66k |
| click-pie | 0/2 · 259 s · 148k | 0/2 · 138 s · 618k | **1/2** · 183 s · 950k | 0/2 · 117 s · 551k |
| build-poll | 2/2 · 114 s · 278k | 2/2 · 116 s · 358k | 2/2 · 165 s · 298k | 2/2 · 213 s · 319k |
| terminal | 2/2 · 49 s · 111k | 2/2 · 44 s · 118k | 2/2 · 58 s · 162k | 2/2 · 71 s · 207k |
| live-github-code-nav | 2/2 · 92 s · 407k | 2/2 · 78 s · 318k | 1/2 · 93 s · 463k | 2/2 · 77 s · 313k |
| live-react-dev-search | 2/2 · 53 s · 186k | 2/2 · 67 s · 257k | 2/2 · 88 s · 341k | 1/2 · 80 s · 314k |
| form-sequence-2 | 0/2 | 0/2 | 0/2 | 0/2 |
| the other 25 tasks | 48/50 | 50/50 | 50/50 | 50/50 |

The full per-run records are in `benchmarks/v016/results/`.

## Where the gains come from

- **Re-rendered lists (key={index}, virtual rows):** unkeyed-inbox went from
  0/4 to 4/4. A reference remembers the row it was shown in and heals to it;
  the click re-aims right before the press. A test clicking across inserts
  with no model: 20/20 (0.1.5: it clicked whatever the node showed).
- **One-call scrolling:** virtual-list fell from 249-400k input tokens and
  86-126 s to 59-78k and 35-40 s; the infinite feed from 262-281k to 170-182k.
- **Rows on repeated buttons:** the agent picks "Dismiss" in "Invoice #4471"
  instead of counting buttons (infinite-feed, unkeyed-inbox, leaderboards).
- **The short MCP guide:** the agents that read the guide no longer go into
  `browser_call` loops (0.1.5: 44 calls and 1.5M tokens on one task).
- **Wrong actions:** 6 -> 0 with GoMini, 2 -> 1 without.

## Where 0.1.6 is not better yet

- **build-poll** took longer on 0.1.6 (165-213 s against 114-116 s). Most of
  it is two `browser_wait` bugs these runs found and the release build fixes:
  the text of a row's context did not match the page's own text (tabs between
  table cells), and `gone` read words such as "Rolling out to production..."
  as a CSS selector, which matched nothing, so the wait said a dialog was gone
  while it was on screen. Both are fixed and checked with no model.
- **terminal** used more tokens (162-207k against 111-118k): the agents tried
  `fill` on the terminal's hidden input before typing at the focus. Not
  changed in 0.1.6.
- **keyed-leaderboard**, once in 4 on 0.1.6 alone: the press landed on a row
  the leaderboard had just moved there, under the load of four browsers and
  four model sessions. The release build re-aims as the last step before the
  press; with no model it scored 20/20 before and after.
- **The live pages** varied from run to run on both versions.
- **The planner still does not delegate.** 0 `browser_delegate` calls in
  every agent run, with GoMini v5 or v6 and the new instructions: GoMini adds
  nothing to these numbers, and its 0.1.6 progress shows only when it is
  given subgoals (below).

Against the targets in the 0.1.6 plan (30% to 90% better). Met: wrong
actions (-100% with GoMini, -50% without), GoMini's held-out practice pages
(+26%, 76.6% -> 96.6%) and the held-out GoMini sweep (+26%, 42 -> 53 of 65;
+36% against 0.1.5's 39), and on the tasks 0.1.6 was built for, tokens and
time by 3-7x (virtual-list, unkeyed-inbox). Not met on the whole mix: success
with GoMini +9% (86.8% -> 94.7%; SPA 82% -> 96%), tokens per solved task -16%
without GoMini and -15% with it (without click-pie), SPA tokens -20%, turns
-14%, median time -10%. Most of these tasks already took four or five turns,
the floor for open, act, answer.

## GoMini v6

### Held-out practice pages (769 decisions, whole pages never trained on)

| | v5 | v6 |
|---|---:|---:|
| all | 76.6% | **96.6%** |
| retry after an error | 33.3% | 100% |
| rows of identical buttons | 58.8% | 100% |
| wait until enabled | 58.6% | 100% |
| focus a box | 59.1% | 100% |
| a covering dialog | 66.7% | 100% |
| collapsed sections | 78.6% | 100% |
| scroll until found | 60.0% | 91.7% |
| "the Nth option" | 58.3% | 75.0% |
| tabs | 80.6% | 75.0% |
| tree | 73.5% | 85.7% |

### MiniWoB++ subgoals (13 held-out task types x 5 seeds, one delegation each)

| | 0.1.5 + v5 | 0.1.6 + v5 | 0.1.6 + v6 |
|---|---:|---:|---:|
| Solved | 39/65 | 42/65 | **53/65** |
| False "done" | 3 | 3 | 4 |
| focus-text | 0/5 | 0/5 | 4/5 |
| click-collapsible | 0/5 | 0/5 | 4/5 |
| social-media | 0/5 | 0/5 | 3/5 |
| click-tab-2 | 2/5 | 2/5 | 2/5 |
| form-sequence-2 | 0/5 | 0/5 | 0/5 |

V6 was trained from v5 on 28,317 decisions: v5's set, 12,049 new teacher
records on 0.1.6's practice pages (eight new families built from the 0.1.5
failures) and 577 steps of gpt-6-luna on 58 MiniWoB++ task types outside every
evaluation set, from runs the page scored as solved. No evaluation page was
used. This build reads v6 from a local folder; the default GoMini download is
still v5 until v6 is published.

## Earlier measurements

Before the final build, each setup ran alone with four workers (results in
`benchmarks/v016/results/`): 0.1.5 + v5 35/38 and 177k tokens per task, 0.1.5
alone 34/38 and 152k, 0.1.6 + v5 33/38 and 116k, 0.1.6 alone 35/38 and 138k.
That run found the regressions the final build fixes (a tree's whole content
read as a row, a click racing a live list) and showed the API drift that made
the setups run together afterwards.

## Reproduce

`benchmarks/` holds the SPA apps and grader, the runner (`v016/gbench.py`),
the four-setup measurement (`v016/phase5.sh`), the GoMini sweep, the no-model
tests and GoMini's practice pages; `benchmarks/README.md` explains each.
