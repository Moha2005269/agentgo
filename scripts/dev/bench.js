// Measures what an agent pays for the common operations and checks the
// documented budgets. Runs on the private desktop like every test:
//   target\debug\agentgo-tool.exe quiet-run -- node scripts/dev/bench.js [--write]
// --write also rewrites docs/benchmarks/performance.md.
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { launch, fixture, repo, refOf, executable } = require("../../tests/integration/harness");

const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const timed = async (fn) => { const started = performance.now(); const value = await fn(); return [performance.now() - started, value]; };
const repeat = async (times, fn) => { const out = []; for (let i = 0; i < times; i += 1) out.push((await timed(fn))[0]); return out; };

// Budgets for a debug build on the development machine; a release build is
// faster. A number over budget fails the run.
const budgets = { startMs: 6000, navigateObserveMs: 1500, observeMs: 150, clickMs: 900, fillMs: 900, screenshotMs: 1200, memoryMb: 1200 };

(async () => {
  const starts = [];
  for (let i = 0; i < 3; i += 1) {
    const [ms, browser] = await timed(async () => { const b = launch({ args: ["--url", "about:blank"] }); await b.call("browser.health"); return b; });
    starts.push(ms);
    await browser.close();
  }
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"] });
  let result;
  try {
    const page = fixture("agent.html");
    const navigate = await repeat(8, () => browser.call("navigate.url", { url: page }));
    const view = await browser.call("observe.page");
    const observe = await repeat(20, () => browser.call("observe.page"));
    const html = (await browser.call("runtime.evaluate", { expression: "document.documentElement.outerHTML.length" })).value;
    const fieldLine = view.page.split("\n").find((line) => /^\[\d+\] textbox/.test(line));
    const field = Number(fieldLine.match(/^\[(\d+)\]/)[1]);
    const fill = await repeat(12, () => browser.call("act.fill", { ref: field, text: "benchmark" }));
    const click = await repeat(12, () => browser.call("act.click", { ref: field }));
    const lastDelta = JSON.stringify(await browser.call("act.fill", { ref: field, text: "delta size" })).length;
    const screenshot = await repeat(5, () => browser.call("observe.screenshot", {}));
    const shot = await browser.call("observe.screenshot", { inline: true });
    const name = path.basename(executable, ".exe");
    const memory = Number(execFileSync("powershell", ["-NoProfile", "-Command",
      `[math]::Round(((Get-Process ${name} -ErrorAction SilentlyContinue | Measure-Object WorkingSet64 -Sum).Sum) / 1MB)`], { encoding: "utf8" }).trim());
    result = {
      startMs: Math.round(median(starts)),
      navigateObserveMs: Math.round(median(navigate)),
      observeMs: Math.round(median(observe)),
      clickMs: Math.round(median(click)),
      fillMs: Math.round(median(fill)),
      screenshotMs: Math.round(median(screenshot)),
      memoryMb: memory,
      htmlChars: html,
      outlineChars: view.page.length,
      deltaChars: lastDelta,
      screenshotBase64Chars: (shot.base64 || "").length,
    };
  } finally {
    await browser.close();
  }
  const over = Object.entries(budgets).filter(([key, limit]) => result[key] > limit);
  console.log(JSON.stringify({ ...result, overBudget: over.map(([key]) => key) }));
  if (process.argv.includes("--write")) {
    const row = (label, key, unit) => `| ${label} | ${result[key]} ${unit} | ${budgets[key]} ${unit} |`;
    fs.writeFileSync(path.join(repo, "docs", "benchmarks", "performance.md"), `# Performance of the common operations

Measured ${new Date().toISOString().slice(0, 10)} by \`scripts/dev/bench.js --write\` on the development
machine, **debug build**, private desktop (window covered, never active), local
fixture page \`tests/fixtures/pages/agent.html\`. Medians. One machine and one
run: use these as the order of magnitude and as a regression check, not as a
claim about other hardware.

| Operation | Median | Budget |
|---|---:|---:|
${row("Start to first answered command", "startMs", "ms")}
${row("navigate.url, loaded and observed", "navigateObserveMs", "ms")}
${row("observe.page", "observeMs", "ms")}
${row("act.click, settled, with change set", "clickMs", "ms")}
${row("act.fill, settled, with change set", "fillMs", "ms")}
${row("observe.screenshot (JPEG)", "screenshotMs", "ms")}
${row("Memory, all processes, one tab", "memoryMb", "MB")}

An action waits until the page has been quiet for about 250 ms before it
reports what changed, so roughly that much of every action time is deliberate.

## What the agent has to read

| Representation of the same page | Characters | About tokens |
|---|---:|---:|
| Full HTML | ${result.htmlChars} | ${Math.round(result.htmlChars / 4)} |
| AgentGo outline (\`observe.page\`) | ${result.outlineChars} | ${Math.round(result.outlineChars / 4)} |
| Whole reply to an action (change set) | ${result.deltaChars} | ${Math.round(result.deltaChars / 4)} |
| One JPEG screenshot, base64 | ${result.screenshotBase64Chars} | image input, typically 1000 or more |

Token counts are characters / 4, an estimate. The comparison with Chrome plus
browser-use on real tasks (success rate, turns, tokens) is the A/B/C experiment
of the graduation roadmap and is **not** measured here.
`);
  }
  process.exit(over.length ? 1 : 0);
})().catch((error) => { console.error(error); process.exit(1); });
