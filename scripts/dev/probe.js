// Dev helper: run protocol calls against a fresh browser and print replies.
//   node scripts/dev/probe.js [--args "<run flags>"] method '{"json":"params"}' method ...
// A method named "sleep" waits params milliseconds.
const { launch, fixture, delay } = require("../../tests/integration/harness");

(async () => {
  const argv = process.argv.slice(2);
  let args = ["--url", "about:blank"];
  if (argv[0] === "--args") { argv.shift(); args = argv.shift().split(" ").filter(Boolean); }
  const browser = launch({ args });
  try {
    for (let i = 0; i < argv.length; i += 2) {
      const method = argv[i];
      const text = (argv[i + 1] || "{}").replace(/FIXTURE:([\w.-]+)/g, (_, name) => fixture(name));
      if (method === "sleep") { await delay(Number(text)); continue; }
      const started = Date.now();
      const reply = await browser.raw(method, JSON.parse(text), 60_000);
      const body = reply.ok ? reply.result : reply.error;
      console.log(`--- ${method} (${Date.now() - started} ms) ${reply.ok ? "ok" : "ERROR"}`);
      console.log(typeof body?.page === "string" ? { ...body, page: "\n" + body.page } : body);
      if (typeof body?.page === "string") console.log(body.page);
    }
  } finally {
    const code = await browser.close();
    if (process.env.PROBE_STDERR) console.error(browser.stderr());
    console.log("exit", code);
  }
})().catch((error) => { console.error(error); process.exit(1); });
