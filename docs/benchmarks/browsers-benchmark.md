# techinz/browsers-benchmark, 2026-10-17

[browsers-benchmark](https://github.com/techinz/browsers-benchmark) loads ten
sites behind bot-protection systems and records whether the real page came
through, plus memory, CPU, a reCAPTCHA v3 score and CreepJS. AgentGo was
driven through its own stdio protocol (an adapter in the style of the
toolkit's engines), Playwright's Chromium 156 (headed) ran as the reference.
One machine (Windows 11, Ryzen, RX 7800 XT), home IP, no proxy, so the
protection results depend on that IP and on this run.

| | AgentGo 0.1.1 (before) | AgentGo 0.1.1 (after fixes) | Playwright Chromium 156 |
|---|---|---|---|
| Sites through (of 10) | 8 | **8** | 7 |
| reCAPTCHA v3 score | 0.9 | 0.9 | 0.9 |
| Average memory | 2172 MB, growing per site | **1410 MB, flat** | 828 MB |
| Average CPU | 27.1 % | 26.6 % | 22.3 % |

| Site (protection) | AgentGo | Chromium |
|---|---|---|
| Google Search | through | blocked |
| community.cloudflare.com (Cloudflare) | through | through |
| datadome.co (DataDome) | challenge | through |
| hermes.com (DataDome) | challenge | challenge |
| Amazon product | through | through |
| Ticketmaster (Imperva) | through | through |
| Mr Porter (Akamai) | through | blocked |
| Priceline (PerimeterX) | through | through |
| Kasada | through | through |
| Reddit | through | through |

DataDome's challenge said the browsing speed looked automated; the run loads
ten protected sites back to back from one IP, and the reference was
challenged on hermes.com the same way.

## What was fixed after the first run

- **Memory** (2172 to 1410 MB): Chromium's back/forward cache kept a whole
  renderer for every site visited (16 processes after ten sites). It is off;
  Back reloads the page.
- **Things a page could see that a normal Chrome does not have**: the ad
  blocker's `window.__agentgoCosmetic` global and the fingerprint marker
  `Symbol.for('agentgo.fp')` are now a random per-run hidden symbol; the
  agent's pointer layer (`<agentgo-stage>`) leaves the page 8 s after the
  agent's last action instead of staying for the page's life; errors thrown
  through the fingerprint protection no longer show its frames in their
  stack.

Already the same as a normal Chrome: `navigator.webdriver` false (the
Playwright reference reports true), no DevTools-protocol console signal,
real GPU and hardware values, protected functions read as native code.

## Known, not fixed

- CreepJS reports "20% stealth" with fingerprint protection on (0% off). Its
  proxy tests (`Object.create(fn).toString()` stack, prototype-cycle and
  `Reflect.setPrototypeOf` behaviour) find the JavaScript Proxies the
  protection is built from. Undetectable noise needs the engine's own C++
  (as Brave does), which means a custom Chromium build.
- The engine is Chromium 152; current Chrome is 156. Updating CEF also
  removes the version gap a site can see.

Reproduce (adapter and probes in `scripts/dev/browsers-benchmark/`): clone the toolkit into a short path, install its requirements and
`playwright install chromium`, then run the adapter
(`run_bench.py <toolkit> <agentgo.exe> --engines agentgo,playwright-chrome`)
under `agentgo-tool quiet-run`.
