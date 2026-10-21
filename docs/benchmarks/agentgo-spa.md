# AgentGo 0.1.5 on single-page apps (SPA benchmark)

The 0.1.5 agent benchmark used MiniWoB++ pages, which are small and mostly
static. This run asks the harder question: what happens on pages with heavy
client-side rendering, where "the page has settled" is the hard part and
element references may not survive aggressive re-renders?

Same machine, model and harness as the published 0.1.5 run
([agentgo-0.1.5.md](agentgo-0.1.5.md), section 5), 2026-10-21.

## Method

- **Agent:** gpt-6-luna (medium reasoning) through Factory's `droid exec`, a
  fresh session for every task, so nothing carries over between tasks or
  setups. Each session gets only one browser's MCP tools (text tools: no
  screenshots, no page script), that server's own instructions, and the same
  prompt: the page URL and the task.
- **Setups:** AgentGo 0.1.5, AgentGo 0.1.5 + GoMini v5 (`browser_delegate`
  on), Playwright MCP 0.0.83, browser-use 0.13.11.
- **Grading:** the 11 local tasks are small single-page apps served locally
  that grade themselves. The page decides the reward, as in MiniWoB++:
  1 = done, -1 = a wrong action (such as the wrong row deleted), no reward =
  never finished. The first consequential action counts, because on a real
  app the wrong item is already gone. The 3 live tasks are read-only public
  pages, graded on the agent's final answer.
- **Measured:** success, tokens (droid's own usage record: input, cached
  input, output), Factory credits, agent turns, browser tool calls, wall time.
  Pages also log diagnostic events (a click before hydration, a click on a
  detached node, an optimistic rollback), and every tool reply is checked
  for stale-reference errors.

### The task set

Each local task targets one thing that is hard on SPAs. Pages are
deterministic (fixed seed, fixed latencies) and use React 18.3.1 where React
behavior matters.

| Task | What it tests |
|---|---|
| skeleton-orders | Skeleton rows, data arrives after 1.8 s, then a background re-sync rebuilds and reorders the whole table with new DOM nodes |
| hydration-cart | Server-rendered buttons that are visible at once but do nothing until the app hydrates 2.5 s after load |
| keyed-leaderboard | React list keyed by id that re-sorts every 1.2 s for 30 s (rows move) |
| unkeyed-inbox | React list with `key={index}`: a new item goes on top every 2 s, so each row's DOM node starts showing a different item |
| virtual-list | Virtualized list: 300 rows, about 20 in the DOM, nodes recycled while scrolling, plus a decoy with the same name |
| spa-router | Client-side routing with `pushState`, per-route spinners, the old view lingering 400 ms during transitions |
| debounced-autocomplete | 350 ms debounce, 0.3 to 1.5 s latency, the suggestion list replaced while typing, options picked on `mousedown` |
| modal-toast | A "What's new" dialog that pops up 2 s after load over the page, a confirm dialog with a typed name, and a toast with a code that disappears after 6 s |
| optimistic-star | Optimistic UI: the star flips at once, the server rejects it 1.2 s later and the UI rolls back; it has to be retried |
| build-poll | A page that never goes still (polling every second, streaming log, moving progress bar); Deploy becomes possible only after about 14 s, and an older build can be deployed at any time (decoy) |
| infinite-feed | Infinite scroll with 1 s loads and no pagination, a decoy handle (@nadia_rr vs @nadia_r) |
| live-todomvc-react | todomvc.com React app: add 3 todos, complete one, filter Active, read the counter |
| live-react-dev-search | react.dev: use the search dialog to open useOptimistic and report its parameters |
| live-github-code-nav | github.com/facebook/react: browse the file tree to `packages/shared/ReactVersion.js` and report the version |

## Results

### All tasks each setup ran

| Setup | Solved | Wrong action | Agent turns | Browser calls | Median s | Mean s | Input tokens | Output tokens | Credits | Input tokens per solved task | Credits per solved task |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| AgentGo 0.1.5 | **12/14** | 2 | **13.4** | **11.3** | **45** | **59** | **185,478** | **687** | **1,405** | **216,391** | **1,640** |
| AgentGo 0.1.5 + GoMini v5 | **12/14** | 2 | 13.7 | 11.5 | 51 | 69 | 189,293 | 897 | 1,484 | 220,842 | 1,732 |
| Playwright MCP 0.0.83 | **12/14** | 2 | 16.0 | 13.6 | 62 | 80 | 274,419 | 754 | 2,034 | 320,156 | 2,373 |
| browser-use 0.13.11 (10 tasks run) | 8/10 | 0 | 21.4 | 17.4 | 67 | 107 | 370,531 | 1,936 | 2,562 | 463,164 | 3,203 |

### The 10 tasks all four setups ran

| Setup | Solved | Wrong action | Agent turns | Browser calls | Median s | Mean s | Input tokens | Credits | Input tokens per solved task |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| AgentGo 0.1.5 | **9/10** | 1 | **11.6** | **9.5** | **43** | **54** | **147,570** | **1,188** | **163,967** |
| AgentGo 0.1.5 + GoMini v5 | **9/10** | 1 | 12.8 | 10.6 | 47 | 68 | 164,486 | 1,332 | 182,762 |
| Playwright MCP 0.0.83 | 8/10 | 2 | 15.3 | 13.0 | 55 | 80 | 228,221 | 1,659 | 285,276 |
| browser-use 0.13.11 | 8/10 | 0 | 21.4 | 17.4 | 67 | 107 | 370,531 | 2,562 | 463,164 |

Input tokens are everything the model read across its turns, most of it
served from the prompt cache.

### Task by task

✓ solved, ✗ failed, then wall time and input tokens.

| Task | AgentGo | AgentGo + GoMini | Playwright MCP | browser-use |
|---|---|---|---|---|
| skeleton-orders | ✓ 37 s · 82k | ✓ 46 s · 132k | ✓ 40 s · 100k | ✓ 43 s · 117k |
| hydration-cart | ✓ 45 s · 94k | ✓ 32 s · 69k | ✓ 39 s · 126k | ✓ 71 s · 212k |
| keyed-leaderboard | ✓ 27 s · 46k | ✓ 34 s · 46k | ✓ 46 s · 73k | ✓ 31 s · 77k |
| unkeyed-inbox | ✗ wrong item · 30 s · 45k | ✗ wrong item · 53 s · 146k | ✗ wrong item · 69 s · 129k | ✓ 63 s · 175k |
| virtual-list | ✓ 151 s · 483k | ✓ 134 s · 288k | ✗ wrong item · 164 s · 808k | ✗ not ended · 139 s · 453k |
| spa-router | ✓ 37 s · 118k | ✓ 38 s · 118k | ✓ 46 s · 167k | ✓ 142 s · 577k |
| debounced-autocomplete | ✓ 41 s · 118k | ✓ 37 s · 81k | ✓ 50 s · 135k | ✓ 41 s · 132k |
| modal-toast | ✓ 50 s · 145k | ✓ 49 s · 146k | ✓ 60 s · 197k | ✓ 75 s · 277k |
| optimistic-star | ✓ 45 s · 96k | ✓ 104 s · 305k | ✓ 64 s · 184k | ✓ 54 s · 184k |
| build-poll | ✓ 71 s · 248k | ✓ 153 s · 313k | ✓ 222 s · 363k | ✗ not ended · 409 s · 1,503k |
| infinite-feed | ✗ wrong item · 80 s · 275k | ✗ wrong item · 76 s · 229k | ✓ 101 s · 369k | not run |
| live-todomvc-react | ✓ 34 s · 106k | ✓ 32 s · 94k | ✓ 58 s · 215k | not run |
| live-react-dev-search | ✓ 58 s · 209k | ✓ 56 s · 197k | ✓ 64 s · 240k | not run |
| live-github-code-nav | ✓ 115 s · 531k | ✓ 124 s · 485k | ✓ 92 s · 736k | not run |

Stale-reference errors returned by the browser: AgentGo 0, AgentGo + GoMini
0, Playwright 2 (`Ref … not found in the current page snapshot`, on
skeleton-orders and build-poll), browser-use 2 (`Element with index … not
found`, on spa-router and build-poll). All four were recovered by reading the
page again. AgentGo's zero is not all good news: see the first loss below.

## Where AgentGo loses

**1. `key={index}` re-renders: the ref followed the node, not the item.**
On unkeyed-inbox, AgentGo read the list, then clicked the Dismiss it had
read for "Invoice #4471 is overdue". In those few seconds a new notification
was inserted on top, React reused that row's node for another notification,
and AgentGo dismissed "Ada mentioned you in #design". The ref was still
valid in AgentGo's sense (the node was connected), so it raised no stale
error. AgentGo + GoMini and Playwright dismissed a wrong item first as well.
browser-use was the only setup that solved it. This is exactly the case in
the question: a ref that survives a re-render while its meaning does not.
Cause: AgentGo checks a ref's fingerprint only after its node is gone, never
while the node is still there.

**2. Repeated buttons without their row.**
On infinite-feed, after scrolling, AgentGo's reply listed the new Like
buttons as `button "♡ Like (18)"` with no post attached, and the posts only
in a separate text read. The model matched buttons to posts by counting and
liked @oatmeal's post. AgentGo + GoMini liked the decoy @nadia_rr. Playwright
solved it, because its snapshot nests each button under its post.

**3. A wrong action reported as done.**
In both losses, AgentGo's change set showed the wrong row changing, and the
agent still answered "Dismissed Invoice #4471" or "Liked @nadia_r's post".
Playwright also claimed success after selecting the wrong row on
virtual-list. A false "done" is worse than a failure, and none of the
browsers catch it today.

**4. Long lists and pages that never go still are expensive.**
AgentGo solved virtual-list, the only setup other than its GoMini variant to
do so, but it took 32 browser calls, 483k input tokens and 151 s. build-poll
took 17 calls and 248k tokens for what is "wait, then press one button".
Both need a single-call "scroll until X" and "wait until X" in AgentGo.

**5. GoMini costs more than it saves on these pages.**
On MiniWoB++, GoMini roughly matched AgentGo alone. Here it was slower and
used more tokens on optimistic-star (21 calls and 305k tokens, against 6 and
96k for AgentGo alone) and build-poll. It has no step for "retry after an
error" or "scroll until found". It did help on virtual-list (18 calls against
32).

## Where it went well

- **Settling:** AgentGo solved skeleton-orders, hydration-cart, spa-router,
  modal-toast, debounced-autocomplete and keyed-leaderboard at the first or
  second try, with the fewest turns. On hydration-cart one click landed before
  hydration (the page logged it). The reply showed no change, and the agent
  clicked again after hydration.
- **Optimistic rollback:** every setup noticed the rollback and retried.
- **Cost:** across the 10 shared tasks, AgentGo used 35% fewer input tokens
  than Playwright MCP and 60% fewer than browser-use per task (42% and 65%
  fewer per solved task), with the fewest turns and the lowest median time.
- **Live pages:** all three solved by AgentGo, GoMini and Playwright.

## Limits and notes

- **Shared load.** The first 6 runs (skeleton-orders for all four setups,
  hydration-cart for GoMini and Playwright) ran one at a time. The rest ran as
  four workers in parallel, one per setup, each with its own browser and
  model session, on one machine. Wall times are therefore higher than in the
  sequential 0.1.5 run and should not be compared with it. Every setup ran
  under the same load, so the comparison here is fair. Tokens, turns and
  success do not depend on load.
- **browser-use was stopped after 10 of its 14 tasks.** It was the slowest
  setup by far (build-poll alone took 409 s, 63 turns and 1.5M input tokens,
  and timed out; its final answer said it had pressed Deploy, but the page
  never registered a deploy). infinite-feed and the 3 live tasks were not run
  for it, so its rows cover 10 tasks; see the 10-task table for a direct
  comparison.
- **One run per task and setup, one seed.** On 14 tasks one task moves a rate
  by 7 points. A different model, or another run of the same one, will move
  single tasks.
- **Payment approval.** In a pilot, the cart's last button was called
  "Checkout", and AgentGo stopped there for human approval: its payment
  safeguard works as designed, also on a test page. The button was renamed to
  "Save cart" so the task measures hydration only. The pilot is not counted.
- **Live pages change.** The expected answers were checked on 2026-10-21
  (React version 19.3.0 on main).

## Reproduce

The apps, the grader and the runner are in `benchmarks/spa/` (see
`benchmarks/README.md`): `spa_server.py` serves and grades the apps,
`tasks.json` holds each goal and answer, `spa_bench.py` runs the four
browsers (`run [--only setup] [--tasks a,b]`, `summary`).

## What 0.1.6 changed

The losses above drove 0.1.6: references that remember their row and heal
after a re-render, rows on repeated buttons, one-call scrolling, waits on
named conditions and `expect` on actions. With them, unkeyed-inbox went from
0/4 to 4/4 and virtual-list from 249-400k to 59-78k input tokens; see
[agentgo-0.1.6.md](agentgo-0.1.6.md).
