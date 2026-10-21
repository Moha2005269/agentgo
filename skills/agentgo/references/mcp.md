# AgentGo over MCP

A real browser. You plan; AgentGo reads pages into short outlines, acts with
its own pointer and keys, waits until the page is quiet, and answers with only
what changed.

## The loop

1. `browser_open {url}`: the reply is the page outline. Lines that start with
   `[n]` can be acted on: `[3] button "Sign in"`.
2. `browser_act {action, ref, ...}`: the reply is a change set (`+` added,
   `~` changed, `-` removed), not the page again. Work from it.
3. `browser_read` only when you need the whole outline again (after a big
   change, or when the reply says a reference is stale). `view: "text"` for
   prose, `filter` to narrow.

## Rules that save turns

- Several steps you are sure of: one `browser_batch {steps: [...]}`.
- Repeated controls carry their row: `[12] button "Dismiss" in "Invoice #4471
  is overdue"`. Pick by the row; never count buttons.
- A long, virtual or endless list: `browser_act {action: "scroll", until:
  "<text>"}` scrolls until that text shows, in one call (add `ref` of anything
  inside the list to scroll that list rather than the page).
- Something that happens later on its own: `browser_wait {text}`, `{gone}`,
  `{enabled: <ref>}` (a button that becomes usable) or `{urlContains}`. It
  returns what changed while waiting; no read is needed after it.
- Waiting after your own actions is built in: do not wait or re-read after
  every click.
- Say what must happen and let the browser check it: `browser_act {action:
  "click", ref: 9, expect: {gone: 9}}` (also `shows: "<text>"`, `enabled:
  <ref>`, `url: "<part>"`, `changes: true`). The reply says `expect: met`, or
  the action fails with what did change, so a click that did nothing cannot
  pass as done.
- The outline beats screenshots; use `browser_screenshot` only for canvases,
  images or layout.
- `browser_call` reaches rare protocol methods (cookies, downloads, PDF,
  network log). Clicking, typing, scrolling and reading never need it.

## Errors

| Code | Do |
|---|---|
| `target.stale` | the element is gone or now shows something else (the message says what): read again, pick again |
| `target.blocked` | something covers it or a dialog is open (the message names it): close or open that first |
| `target.ambiguous` | pick one of the listed references |
| `approval.required`, `handoff.required` | `browser_wait_human` (payments, final submits, CAPTCHAs are the person's) |
| `policy.denied` | tell the person; do not work around it |

## GoMini

With `browser_delegate` present, GoMini (a small model on this computer) can
take a routine multi-step subgoal: give `subgoal` (one goal, named as the page
names things), `values` (every text to type, by name) and `completion`. Worth
it for runs of steps you would otherwise read and act through one by one:
opening tabs, folders or sections until something shows, ticking a set of
boxes, filling a long form, retrying a step the page rejected. Not worth it for
one click. `done` is GoMini's claim: glance at the evidence. `needs_help`:
do that step yourself.
