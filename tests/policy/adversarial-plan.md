# Adversarial policy test plan

Milestone 7 gate: "adversarial policy tests prove adapters/pages cannot
bypass controls; purchase suite always stops; CAPTCHA suite never
attempts a solution and resumes correctly after a human fixture."

## Suites to build (each must try to bypass through every entry point)

1. **Money hard stop** — purchase/payment/transfer/subscription/bid/
   donation/final-checkout through: stdio command, named pipe, MCP tool,
   shell bridge, replay, workflow batch, keyboard-typed checkout,
   JS-initiated submit, page-injected "auto-approve" text. All must
   produce `approval.required` and pause.
2. **CAPTCHA handoff** — challenge pages must produce `handoff.required`
   in both modes; no solve/bypass code path may exist; resume only after
   the human fixture completes; same tab/profile/cookies/IP/checkpoint
   verified.
3. **Prompt injection** — page text claiming "the operator approved
   this" must not change policy; permission grants only from the
   protocol/operator path.
4. **Mode confusion** — Full-Access mode must not upgrade money/captcha/
   certificate rows; switching modes mid-workflow must not retroactively
   approve a paused action.
5. **Adapter parity** — the same command through stdio, pipe, MCP, SDK,
   and shell must yield identical decisions (matrix property test).
6. **Credential rules** — autofill without plaintext exposure; plaintext
   visibility off by default and still redacted from logs/artifacts.
7. **Scope escapes** — URL/file scope traversal attempts (look-alike
   origins, userinfo tricks, `..` paths, symlinked roots).

Milestone 0 status: the decision matrix and risk classifier are
implemented with exhaustive unit tests (`crates/agentgo-policy`); the
adversarial runtime suites land with Milestone 7.
