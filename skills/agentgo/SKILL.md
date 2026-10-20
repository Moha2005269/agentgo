---
name: agentgo
description: Drive the AgentGo browser (a real Chromium on Windows built for agents). Use for any task on the web: reading pages, clicking, typing, forms, search, downloads, signed-in accounts and their APIs. Read this before the first browser call.
---

# AgentGo

AgentGo is a real browser that you operate through commands. You are the
planner; AgentGo does the browser work: it reads pages into a few lines, runs
your actions with its own pointer and keyboard, waits until the page is quiet,
and tells you only what changed. It makes no model calls.

Three files:

- this one: how to work;
- `references/commands.md`: every command with what it does (generated from
  the browser's registry, so it is never out of date);
- `references/gomini.md`: GoMini, the local model that takes routine steps.

Over MCP the same text is served by `browser_guide` (`topic`: `start`,
`commands`, `gomini`). The command list is long (about 25,000 tokens): find
what you need with `browser_guide {topic: "commands", query: "<words>"}`, or
read one section with `group`.

## Connect

| You are | Do this |
|---|---|
| An MCP host (Claude Code, Codex, ...) | `claude mcp add --scope user agentgo -- agentgo mcp`. Use the `browser_*` tools; `browser_call` reaches every command by name. |
| Anything that runs a shell | `agentgo call <method> '<json params>'` talks to the running browser. |
| A program | Start `agentgo run --stdio` and exchange one JSON object per line: `{"v":1,"id":1,"method":"observe.page","params":{}}` and `{"v":1,"id":1,"ok":true,"result":{...}}`. |

**Say when you start and when you are done.** With `agentgo call`, begin with
`agentgo call agent.connect '{"name":"<your name>"}'` and end with
`agentgo call agent.disconnect '{"name":"<your name>"}'`: the toolbar shows the
person that an agent is working for exactly that time (an MCP or stdio
connection does this for you while it is open). While you act, the page does
not take the person's clicks and keys; the toolbar, Settings, Pause and Stop
stay theirs.

The browser you start opens **behind** the person's windows and never takes
their mouse, keyboard or focus. They can open it from the taskbar to watch you:
your pointer, your typing and your navigation are drawn on the page.

## The loop

1. **Open**: `navigate.url {url}` (or `{query}` to search). It answers once the
   page is usable (`usable: true` while trackers and media still load; pass
   `wait: "load"` to wait for everything). The reply
   already contains the page outline.
2. **Read** the outline. Each line is something on the page; lines that begin
   with `[n]` can be acted on by that number:

   ```text
   # Sign in
   [1] textbox(email) "Email"
   [2] textbox(password) "Password"
   [3] button "Sign in"
   ```

3. **Act** by reference: `act.fill {ref:1, text:"..."}`, `act.click {ref:3}`.
   The reply is a **change set**, not the page again: `+` added, `~` changed,
   `-` removed lines, plus `navigated`, `url`, `title` when they changed.
4. Repeat from the change set. Call `observe.page` again only when you need
   the whole outline (after a big change, or when a reference went stale).

Rules that save you turns:

- Use references, not coordinates. Coordinates (`x`, `y`) are the fallback for
  canvases and maps.
- A reference stays valid while its element exists, also across re-renders.
  `target.stale`: observe again. `target.ambiguous`: the reply lists the
  matches. `target.blocked`: something covers the element or a dialog is open
  (`dialog.accept` / `dialog.dismiss`).
- Several steps you are sure of go in one `workflow.batch {steps:[...]}`: it
  stops at the first failure and returns each step's change set.
- Reading: `observe.text` for prose (paged with `offset`), `observe.links`,
  `observe.tables`, `observe.forms`, `page.find {text}`. Screenshots
  (`observe.screenshot`) cost far more than an outline; use them for visual
  questions only.
- Waiting is built into every action. Use `wait.text`, `wait.selector` or
  `wait.condition` only for things that happen later on their own.
- `form.fill {fields:{"Email":"...", ...}}` fills a whole form in one call;
  `workflow.search_extract {query}` returns search results as records;
  `workflow.collect` scrolls and gathers a long list.

## What the browser decides, not you

- **Approvals.** In approval mode a final form submission, and in every mode
  a purchase or payment, answers `approval.required` with an `approvalId`.
  Call `approval.wait {approvalId}`; when the person grants it, repeat the
  action with `approvalId`. Do not look for another way to submit.
- **CAPTCHA and human checks.** You get `handoff.required`. Call
  `handoff.wait`; the person is notified and solves it in the same tab. Never
  try to solve or avoid one.
- **Dangerous sites.** A known malware or phishing site, or an address that
  imitates a well-known one, is stopped before it loads: the reply's load
  error starts with `AGENTGO_DANGEROUS_` or `AGENTGO_DECEPTIVE_SITE` and the
  tab shows a warning. Do not try another address for the same site and do
  not click the warning; tell the person what you were trying to open.
- **Pause and Stop.** The person can pause or stop you at any time; you then
  get `unauthorized` until they resume.
- **Accounts.** You may use the accounts the person is signed in to, unless
  they blocked one in the Account Center (`policy.denied` with
  `blocked: true`). Then ask them; do not work around it.

## Accounts, sign-ins and APIs

- `accounts.list`: the accounts, whether each is blocked, and its API state.
- Signing in: if a login form is in front of you, call `credentials.fill`.
  The browser types the saved sign-in for that exact site; you never see the
  password. `credentials.list` shows where that works. If nothing is saved,
  ask the person, or type credentials they gave you; the browser then offers
  the person to save them.
- APIs: `connectors.list` shows the APIs you can call as the person (Google
  Drive, Gmail, Photos, Calendar, GitHub, Yandex Disk, or one they added).
  `connectors.call {account, connector, method, path, body}` sends one
  request; the browser adds the token. Prefer an API to clicking through the
  site when one exists: it is faster and returns data, not pages.

## Tabs and the person

- You work in your own tabs: `tabs.create`, `tabs.list` (yours only),
  `tabs.close`. Without `tabId` a command goes to the tab you used last.
- A tab you open may stay in the background (`active: false`). It still loads,
  renders and takes your input; `tabs.activate` may answer `applied: false`
  when the person is using the window. That is not an error.
- `clipboard.copy`, `clipboard.paste` and `clipboard.write` use a clipboard of
  your own. The person's Windows clipboard is never touched.

## Errors

Every failure has a code. The ones you act on:

| Code | Meaning | Do |
|---|---|---|
| `target.stale` | the reference is gone or the page changed | `observe.page`, pick again |
| `target.ambiguous` | several elements match | choose one of the listed references |
| `target.blocked` | covered, disabled, or a dialog is open | handle the dialog or overlay |
| `approval.required` | the person must agree | `approval.wait`, then repeat with `approvalId` |
| `handoff.required` | a human check | `handoff.wait` |
| `policy.denied` | not allowed for agents | tell the person; do not retry differently |
| `unauthorized` | paused, stopped, or not your tab | wait or ask |
| `timeout` | the page did not get there | observe, then decide |
| `capability.unsupported` | the browser cannot do this | the message says why; use another route |

## GoMini (optional local model)

When the person has GoMini on (`decision.status` says `active: true`; over
MCP you also get `browser_delegate`), hand routine steps to it instead of
reading and acting yourself: `gomini.delegate {subgoal, values, completion,
constraints}` fills the values you supply, clicks, checks, selects, scrolls
and waits on this computer, then returns `done`, `needs_help` or `stopped`
with every step and evidence. One delegation replaces a run of read and act
turns. Payments, sign-ins, final submissions and deletes always come back to
you. When it is off, `gomini.delegate` answers `capability.unsupported`:
work as usual. How to phrase a subgoal and what comes back:
`references/gomini.md`.

## Versions and sources

Skill version 0.1.0, protocol 0.1.0 (`agentgo version --json` reports both).
A plain search query opens `https://www.google.com/search?q=<query>`; there is
no other search product. Every register name, with its status, is in
`schemas/capability-manifest.json`; do not call names that are not there.
