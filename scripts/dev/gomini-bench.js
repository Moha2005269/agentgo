// How much an agent's model reads and writes, and how many turns it takes,
// to finish the same form tasks four ways:
//   A  raw HTML       what a generic automation tool hands the model
//   B1 AgentGo        compact outline, one action per turn
//   B2 AgentGo batch  one read, then every action in one browser_batch
//   C  GoMini         one browser_delegate; GoMini takes the steps locally
// Tokens are estimated as characters / 4 of what the model would see over
// MCP (results rendered exactly as AgentGo's MCP server renders them).
//   AGENTGO_GOMINI_ROOT=<installed GoMini> node scripts/dev/gomini-bench.js [tasks]
const fs = require("node:fs");
const path = require("node:path");
const { launch, serve, reservePort, operator, repo } = require("../../tests/integration/harness");

const TASKS = Number(process.argv[2] || 24);
const tokens = (text) => Math.ceil(String(text).length / 4);

// AgentGo's MCP rendering (crates/agentgo-mcp/src/server.rs `render`).
function render(result) {
  if (!result || typeof result !== "object" || Array.isArray(result)) return JSON.stringify(result);
  const out = [];
  const rest = { ...result };
  const view = ["page", "text", "changes"].some((k) => k in result);
  if (!view && rest.tabId != null) { out.push(`tabId: ${rest.tabId}`); delete rest.tabId; }
  const take = (k) => { const v = rest[k]; delete rest[k]; return v == null ? undefined : v; };
  const url = take("url"), title = take("title");
  if (url !== undefined) out.push(`${title || ""} — ${url}`.replace(/^ — /, ""));
  for (const k of ["page", "text", "changes", "filled"]) {
    const v = take(k);
    if (typeof v === "string") out.push(k === "changes" ? (v ? `changed:\n${v}` : "(no visible change)") : v);
  }
  for (const k of ["tabId", "scroll", "total", "ms", "accepted", "loaded", "base64"]) { const v = take(k); if (k === "loaded" && v === false) out.push("(the page had not finished loading)"); }
  const next = take("next");
  if (next !== undefined) out.push(`(more: pass offset ${next})`);
  if (Object.keys(rest).length) out.push(JSON.stringify(rest));
  return out.join("\n");
}

const refOf = (page, label) => {
  const line = page.split("\n").find((l) => l.includes(`"${label}"`) && /^\[\d+\]/.test(l));
  return line ? Number(line.match(/^\[(\d+)\]/)[1]) : null;
};

(async () => {
  const site = await serve();
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"], debugPort: port, env: { AGENTGO_GOMINI_ROOT: process.env.AGENTGO_GOMINI_ROOT || "" } });
  const rows = [];
  try {
    const status = await browser.call("gomini.status");
    if (!status.installed) throw new Error("GoMini is not installed at AGENTGO_GOMINI_ROOT");
    await operator(port, "settings.set", { settings: { gominiEnabled: true } });
    for (let i = 0; i < 100 && !(await browser.call("gomini.status")).running; i += 1) await new Promise((r) => setTimeout(r, 200));
    for (let seed = 1; seed <= TASKS; seed += 1) {
      const url = site.url(`gomini-bench.html?seed=${seed}`);
      const open = async () => { await browser.call("navigate.url", { url }); return (await browser.call("runtime.evaluate", { expression: "JSON.stringify(window.task)" })).value; };
      const task = JSON.parse(await open());
      const saved = async () => (await browser.call("decision.record", { subgoal: "check" })).state.status.includes("Draft saved successfully.");
      const published = async () => /published/.test((await browser.call("runtime.evaluate", { expression: "JSON.stringify(window.events)" })).value);
      const result = { seed, domain: task.domain, fields: task.fields.length };

      // A: raw HTML each time the model looks; same steps as B1.
      {
        let read = 0, written = 0, turns = 0;
        const html = async () => (await browser.call("runtime.get_html", { selector: "html", max: 400000 })).html;
        const page = (await browser.call("observe.page")).page;
        read += tokens(await html()); turns += 1;
        for (const f of task.fields) {
          const args = f.list ? { action: "select", ref: refOf(page, f.label), value: f.value } : { action: "fill", ref: refOf(page, f.label), text: f.value };
          written += tokens(JSON.stringify(args));
          await browser.call(f.list ? "act.select_option" : "act.fill", args);
          read += tokens(await html()); turns += 1;
        }
        written += tokens(JSON.stringify({ action: "click", ref: refOf(page, "Save draft") }));
        await browser.call("act.click", { ref: refOf(page, "Save draft") });
        read += tokens(await html()); turns += 1;
        result.A = { read, written, turns, ok: await saved() };
      }
      // B1: AgentGo outline, one action per turn.
      {
        await open();
        let read = 0, written = 0, turns = 0, ms = Date.now();
        const view = await browser.call("observe.page");
        read += tokens(render(view)); turns += 1;
        for (const f of task.fields) {
          const method = f.list ? "act.select_option" : "act.fill";
          const args = f.list ? { action: "select", ref: refOf(view.page, f.label), value: f.value } : { action: "fill", ref: refOf(view.page, f.label), text: f.value };
          written += tokens(JSON.stringify(args));
          read += tokens(render(await browser.call(method, args))); turns += 1;
        }
        const click = { action: "click", ref: refOf(view.page, "Save draft") };
        written += tokens(JSON.stringify(click));
        read += tokens(render(await browser.call("act.click", click))); turns += 1;
        result.B1 = { read, written, turns, ok: await saved(), ms: Date.now() - ms };
      }
      // B2: one read, one batch.
      {
        await open();
        let read = 0, written = 0, ms = Date.now();
        const view = await browser.call("observe.page");
        read += tokens(render(view));
        const steps = task.fields.map((f) => f.list ? { action: "select", ref: refOf(view.page, f.label), value: f.value } : { action: "fill", ref: refOf(view.page, f.label), text: f.value });
        steps.push({ action: "click", ref: refOf(view.page, "Save draft") });
        written += tokens(JSON.stringify({ steps }));
        const mapped = steps.map((s) => ({ method: s.action === "select" ? "act.select_option" : s.action === "fill" ? "act.fill" : "act.click", params: s }));
        read += tokens(render(await browser.call("workflow.batch", { steps: mapped })));
        result.B2 = { read, written, turns: 2, ok: await saved(), ms: Date.now() - ms };
      }
      // C: one delegation to GoMini.
      {
        await open();
        const values = Object.fromEntries(task.fields.map((f, i) => [`v${i + 1}`, f.value]));
        const args = {
          subgoal: `Fill this ${task.domain} form using ${task.fields.map((f, i) => `${f.label}=v${i + 1}`).join("; ")}. Save draft when all required fields match. Never publish.`,
          values, completion: "Draft saved successfully with every requested field value", constraints: ["Never publish. Never invent a missing value."],
        };
        const ms = Date.now();
        const out = await browser.call("gomini.delegate", args, 120_000);
        result.C = { read: tokens(render(out)), written: tokens(JSON.stringify(args)), turns: 1, ok: (await saved()) && out.status === "done", status: out.status, reason: out.reason, steps: out.steps.length, modelMs: out.modelMs, ms: Date.now() - ms, published: await published() };
      }
      rows.push(result);
      console.log(JSON.stringify(result));
    }
  } finally {
    await browser.close();
    site.close();
  }
  // Summary.
  const modes = ["A", "B1", "B2", "C"];
  const sum = (mode, key) => rows.reduce((t, r) => t + (r[mode][key] || 0), 0);
  const okCount = (mode) => rows.filter((r) => r[mode].ok).length;
  const lines = [
    `# GoMini usage benchmark`, "",
    `${rows.length} form tasks (\`tests/fixtures/pages/gomini-bench.html\`, seeds 1-${rows.length}): 2-4 fields, some lists, 18-68 other links and filler text per page. Tokens are characters / 4 of what the agent's model reads (tool results as AgentGo's MCP server renders them) and writes (tool arguments). Run with \`node scripts/dev/gomini-bench.js\`.`, "",
    `| Mode | Model reads (tokens, total) | Model writes | Turns (total) | Tasks done | Browser time |`,
    `|---|---|---|---|---|---|`,
    ...modes.map((m) => `| ${{ A: "A raw HTML", B1: "B1 AgentGo, step by step", B2: "B2 AgentGo, one batch", C: "C GoMini, one delegation" }[m]} | ${sum(m, "read").toLocaleString()} | ${sum(m, "written").toLocaleString()} | ${sum(m, "turns")} | ${okCount(m)}/${rows.length} | ${m === "A" ? "—" : `${Math.round(sum(m, "ms") / rows.length)} ms avg`} |`),
    "",
    `- AgentGo alone (B1 vs A): the model reads ${(sum("A", "read") / Math.max(1, sum("B1", "read"))).toFixed(1)}x less.`,
    `- GoMini (C vs B1): ${(sum("B1", "read") / Math.max(1, sum("C", "read"))).toFixed(1)}x less to read and ${(sum("B1", "turns") / Math.max(1, sum("C", "turns"))).toFixed(1)}x fewer turns; vs the best manual use (B2): ${(sum("B2", "read") / Math.max(1, sum("C", "read"))).toFixed(1)}x less to read, ${(sum("B2", "turns") / Math.max(1, sum("C", "turns"))).toFixed(1)}x fewer turns.`,
    `- GoMini model time: ${Math.round(sum("C", "modelMs") / rows.length)} ms per task (${rows.reduce((t, r) => t + r.C.steps, 0)} decisions in all). Each turn saved is one fewer round trip to the agent's large model, which takes seconds.`,
    `- Scope: these are AgentGo form pages of the kind GoMini was trained on; they show the savings and that the checks hold, not GoMini's accuracy on arbitrary websites. Characters / 4 is an estimate of tokens; the ratios between modes are what matter.`,
    `- GoMini never pressed Publish: ${rows.every((r) => !r.C.published) ? "yes" : "NO"}. Not done: ${rows.filter((r) => !r.C.ok).map((r) => `seed ${r.seed} (${r.C.status}: ${r.C.reason || ""})`).join(", ") || "none"}.`,
    "", "Raw rows:", "", "```", ...rows.map((r) => JSON.stringify(r)), "```",
  ];
  const out = path.join(repo, "docs", "benchmarks", "gomini-usage.md");
  fs.writeFileSync(out, lines.join("\n") + "\n");
  console.log(lines.slice(0, 14).join("\n"));
})().catch((error) => { console.error(error); process.exit(1); });
