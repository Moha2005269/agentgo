// Runs the real-browser integration tests one after another and prints a
// PASS/FAIL table. Each test launches its own browser, so they must not run in
// parallel. Usage:
//   node scripts/dev/run-integration.js                 (all offline tests)
//   node scripts/dev/run-integration.js agent_e2e sessions_e2e
//   node scripts/dev/run-integration.js --live          (also the tests that
//                                                        need the internet)
// Output of every test is kept in target/integration-logs/<name>.log.
//
// Every test runs on a private Win32 desktop (`agentgo-tool quiet-run`), so
// no browser window ever appears on the screen, takes focus or receives the
// user's input, and the run fails if one does. Set AGENTGO_TEST_VISIBLE=1 to
// watch a test on your own desktop on purpose.
const { spawn, execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const repo = path.resolve(__dirname, "../..");
const testsDir = path.join(repo, "tests", "integration");
const logDir = path.join(repo, "target", "integration-logs");
const tool = path.join(repo, "target", "debug", "agentgo-tool.exe");
const visible = process.env.AGENTGO_TEST_VISIBLE === "1";
const TIMEOUT_MS = Number(process.env.AGENTGO_TEST_TIMEOUT_MS || 300_000);
const args = process.argv.slice(2);
const includeLive = args.includes("--live") || process.env.AGENTGO_LIVE === "1";

// A test that loads real third-party sites says so with an `@live` marker in
// its header: its result depends on the network and on sites it does not own,
// so it never belongs in the default, deterministic run.
function isLive(name) {
  const head = fs.readFileSync(path.join(testsDir, `${name}.js`), "utf8").split("\n").slice(0, 6).join("\n");
  return /@live\b/.test(head);
}

// protocol_smoke first (it fails fastest when the build is broken), then the
// rest alphabetically.
function allTests() {
  const found = fs.readdirSync(testsDir)
    .filter((file) => /_(e2e|smoke)\.js$/.test(file))
    .map((file) => file.replace(/\.js$/, ""))
    .sort();
  return ["protocol_smoke", ...found.filter((name) => name !== "protocol_smoke")]
    .filter((name) => found.includes(name))
    .filter((name) => includeLive || !isLive(name));
}

function leftoverBrowsers() {
  try {
    const out = execFileSync("powershell", ["-NoProfile", "-Command",
      "(Get-Process agentgo -ErrorAction SilentlyContinue | Measure-Object).Count"], { windowsHide: true, encoding: "utf8" });
    return Number(out.trim()) || 0;
  } catch (_) { return -1; }
}

function commandFor(file) {
  if (visible) return [process.execPath, [file]];
  return [tool, ["quiet-run", "--report", "--", process.execPath, file]];
}

function run(name) {
  return new Promise((resolve) => {
    const file = path.join(testsDir, `${name}.js`);
    if (!fs.existsSync(file)) { resolve({ name, status: "SKIP", ms: 0, note: "no such test" }); return; }
    const started = Date.now();
    const log = fs.createWriteStream(path.join(logDir, `${name}.log`));
    const [program, args] = commandFor(file);
    const child = spawn(program, args, { cwd: repo, env: process.env, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    let tail = "";
    let all = "";
    const keep = (chunk) => { log.write(chunk); all += chunk; tail = (tail + chunk).slice(-1500); };
    child.stdout.on("data", keep);
    child.stderr.on("data", keep);
    let timedOut = false;
    const timer = setTimeout(() => { timedOut = true; child.kill(); }, TIMEOUT_MS);
    child.on("error", (error) => keep(`failed to start ${program}: ${error.message}\n`));
    child.on("exit", (code) => {
      clearTimeout(timer);
      log.end();
      const quiet = all.match(/quiet-run: exit \d+ \| (\d+) process\(es\) \| peak (\d+) visible window\(s\) on the private desktop \| (\d+) on your desktop/);
      resolve({
        name,
        status: code === 0 && !timedOut ? "PASS" : "FAIL",
        ms: Date.now() - started,
        quiet: quiet ? `peak ${quiet[2]} window(s) on the private desktop, ${quiet[3]} on yours` : "",
        note: timedOut ? "timed out" : code === 0 ? "" : tail.trim().split("\n").slice(-6).join("\n"),
      });
    });
  });
}

(async () => {
  fs.mkdirSync(logDir, { recursive: true });
  if (!visible && !fs.existsSync(tool)) {
    console.error(`missing ${tool}\nbuild it first: powershell -File scripts\\build.ps1 -Runtime`);
    process.exit(2);
  }
  const wanted = args.filter((arg) => !arg.startsWith("--"));
  const names = wanted.length ? wanted : allTests();
  console.log(visible ? "running on YOUR desktop (AGENTGO_TEST_VISIBLE=1)" : "running on a private desktop (quiet): nothing will appear on screen");
  const results = [];
  for (const name of names) {
    process.stdout.write(`running ${name} ... `);
    const result = await run(name);
    results.push(result);
    console.log(`${result.status} (${(result.ms / 1000).toFixed(1)}s)${result.quiet ? ` [${result.quiet}]` : ""}`);
    if (result.status === "FAIL") console.log(result.note.split("\n").map((l) => `    ${l}`).join("\n"));
  }
  const failed = results.filter((r) => r.status === "FAIL");
  console.log(`\n${results.length - failed.length}/${results.length} passed`);
  const left = leftoverBrowsers();
  if (left > 0) console.log(`warning: ${left} agentgo.exe process(es) still running after the tests`);
  process.exit(failed.length ? 1 : 0);
})();
