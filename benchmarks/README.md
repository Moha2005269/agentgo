# Benchmark harness

What `docs/benchmarks/agentgo-spa.md` and `docs/benchmarks/agentgo-0.1.6.md`
were measured with. One model drives every setup: gpt-6-luna (medium
reasoning) through Factory's `droid exec`, a fresh session per task, with only
the browser's MCP tools (text tools: no screenshots, no page script). The page
decides success.

## spa/: single-page apps that grade themselves

- `spa_server.py` serves the 11 apps in `pages/` (React 18.3.1 where React's
  behaviour matters) and grades them: `GET /spa/<task>?seed=&run=`,
  `GET /result?run=` returns the page's reward and its diagnostic events
  (clicks before hydration, rows dismissed, optimistic rollbacks...).
- `tasks.json`: goal and answer of each task. `spa_bench.py`: the four-browser
  runner used for the SPA report; it also holds the three live pages.

| Task | What it tests |
|---|---|
| skeleton-orders | skeleton rows, late data, a background re-sync that rebuilds and reorders the table |
| hydration-cart | server-rendered buttons that do nothing until the app hydrates (2.5 s) |
| keyed-leaderboard | React keyed list that re-sorts every 1.2 s |
| unkeyed-inbox | React `key={index}` list with inserts on top: nodes change content under the agent |
| virtual-list | 300 rows, about 20 in the DOM, nodes recycled while scrolling |
| spa-router | `pushState` routing, per-route spinners, the old view lingering |
| debounced-autocomplete | debounce, variable latency, suggestions replaced while typing |
| modal-toast | a late covering dialog, a typed-name confirm, a toast that disappears |
| optimistic-star | optimistic UI that rolls back after a server error |
| build-poll | a page that never goes still; an action possible only later |
| infinite-feed | infinite scroll with 1 s loads, a decoy handle |

## v016/: AgentGo with and without GoMini

- `gbench.py`: the runner: suites `mw-eval` (the 24 MiniWoB++ tasks of the
  0.1.5 report), `spa` (the 14 SPA tasks) and `mw-diag` (58 other MiniWoB++
  tasks: diagnostics and GoMini training data, never an evaluation set);
  `--pool` picks the bridges (gw/gs: 0.1.6 with/without GoMini, bw/bs: 0.1.5).
  Every run records tokens, turns, tool calls, time, the page's reward, stale
  references and its slice of the GoMini trace.
- `mcpd.py` keeps one browser MCP server alive behind a local port and, with
  `AGENTGO_RECORD`, saves the agent's clicks, fills, checks and selects with
  GoMini's candidates for the page (training records labelled by the agent).
  `mcp_proxy.py` is what the agent host connects to.
- `phase5.sh`: the four setups at the same time, one worker each, so they
  share the model API's conditions (the final numbers).
- `sweep_gomini.py`, `sweep.sh`: GoMini alone on 13 held-out MiniWoB++ task
  types x 5 seeds, one delegation per task.
- `smoke.py`: no model; checks the 0.1.6 behaviours on the SPA and MiniWoB++
  pages (a ref across a `key={index}` insert, scroll until, wait until
  enabled, a timed pause in a batch, scrolling inside a textarea, `expect`).
- `practice-pages.html`: GoMini's practice families (v5's and v6's) with their
  answer keys (`window.__gold()`), for the teacher that records training data.
- `results/`: the summaries behind the reports.

The scripts were run from `C:\Users\m7md0\bench` (MiniWoB++ pages from
BrowserGym's `browsergym-miniwob` 0.14.3 on port 18765, the SPA server on
18780); paths in them point there.
