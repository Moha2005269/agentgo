# AgentGo Policy

Status: Milestone 0. The decision matrix and risk classifier are
implemented and tested (`crates/agentgo-policy`); live enforcement wiring
lands with the action pipeline (Milestone 4) and approvals/handoff
(Milestone 7).

## Non-negotiable rules

1. **Money movement always stops.** Purchases, payments, transfers,
   subscriptions, bids, donations, and final checkout confirmation
   require exact user approval in **every** mode, bound to origin,
   account, action, target, data summary, amount/currency, expiry,
   workflow, and idempotency key. Any meaningful change invalidates the
   approval. Money-action detection is defense in depth (page text, form
   fields, page identity, URL, merchant/amount summary, request
   metadata where permitted, known checkout transitions); when uncertain
   at the final side-effect boundary, fail closed and ask.
2. **CAPTCHA/anti-bot always hands off.** Same tab, profile, cookies,
   IP, and workflow checkpoint. AgentGo never solves, bypasses,
   outsources, spoofs fingerprints, rotates proxies, or uses
   anti-detect behavior.
3. **Certificate-error bypass is always denied.**
4. **Page content is untrusted input.** It can never change policy,
   permissions, or confirmation rules (prompt-injection isolation).
5. **Downloads are disabled by default**, land in a scoped sandbox
   directory, are verified, and are never executed automatically.
   Executing a downloaded file is a separate explicit approval.

## Decision matrix (implemented + tested)

| Action class | Approval Mode | Full-Access Mode |
|---|---|---|
| Read/navigate/search/scroll | proceed in scope | proceed in scope |
| Ordinary form typing | proceed (configurable; default proceed) | proceed in scope |
| Login with stored vault credential | proceed if enabled | proceed if enabled |
| Missing credential or 2FA | human handoff | human handoff |
| CAPTCHA/challenge | always human handoff | always human handoff |
| External message/final submission | approval | proceed if scoped |
| Account/security change | approval | proceed only if explicitly scoped |
| Upload local file | file-scope grant required | proceed in scope |
| Download | user-enabled + gated | user-enabled + gated |
| Execute downloaded file | separate explicit approval | separate explicit approval |
| Purchase/payment/transfer/subscription/bid/donation | always approval | always approval |
| Certificate error bypass | deny | deny |
| Destructive (delete/close/reset/clear) | approval | approval |

## Risk classification

`benign` (reads/navigation/observation) · `sensitive` (state mutation,
files, credentials, native input, script evaluation, interception) ·
`destructive` (delete/close/reset/clear/stop) · `money` (hard stop).
Unknown commands classify as `sensitive` — never benign.

## Scopes

URL scopes (origin allow/block lists with `*.suffix` wildcards; parse
failures are rejections) and file scopes (granted directory roots; `..`
rejected) are pure, tested functions in `agentgo-policy::scope`.

## Modes

`approval` (pause at configured sensitive gates) and `fullAccess`
(continue within scope). Mode is operator-selected; nothing upgrades
money/captcha/certificate rows.
