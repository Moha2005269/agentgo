// @live
// Real-world tasks on public websites AgentGo does not control, done the way
// an agent would: read the outline, pick references, act, check the result.
// Each scenario is recorded (ok, time, what was seen) instead of stopping at
// the first failure, so one run shows everything that breaks.
//   node scripts/dev/run-integration.js --live real_web_e2e
// Writes target/real-web-report.json.
const fs = require("node:fs");
const path = require("node:path");
const { launch, repo } = require("./harness");

const lineOf = (page, test) => page.split("\n").find((line) => /^\[\d+\]/.test(line) && test(line));
const matcher = (test) => typeof test === "function" ? test : test instanceof RegExp ? (l) => test.test(l) : (l) => l.includes(test);
const ref = (page, test, what) => {
  const line = lineOf(page, matcher(test));
  if (!line) throw new Error(`no ${what || test} in the outline:\n${page.split("\n").slice(0, 40).join("\n")}`);
  return Number(line.match(/^\[(\d+)\]/)[1]);
};
const check = (condition, message) => { if (!condition) throw new Error(message); };

(async () => {
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"] });
  const call = (method, params = {}, timeout = 45_000) => browser.call(method, params, timeout);
  // As an agent should: a site that has not answered yet is waited for.
  const open = async (url) => {
    const view = await call("navigate.url", { url, timeout: 30_000 });
    if (view.loaded !== false) return view;

    console.log(`      (${url} still loading after ${view.ms} ms; waiting)`);
    await call("wait.load", { timeout: 45_000 }, 60_000);
    return call("observe.page");
  };
  const report = [];
  // A site that stalls must not hold the run: each scenario has a deadline.
  const DEADLINE_MS = 90_000;
  const scenario = async (name, body) => {
    const started = Date.now();
    const notes = [];
    try {
      let timer;
      await Promise.race([
        body(notes),
        new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`scenario exceeded ${DEADLINE_MS / 1000} s`)), DEADLINE_MS); }),
      ]).finally(() => clearTimeout(timer));
      report.push({ name, ok: true, ms: Date.now() - started, notes });
      console.log(`ok    ${name} (${Date.now() - started} ms)`);
    } catch (error) {
      const url = await call("tabs.status").then((s) => s.url, () => "?");
      report.push({ name, ok: false, ms: Date.now() - started, error: `${error.code || ""} ${error.message}`.trim().slice(0, 1500), url, notes });
      console.log(`FAIL  ${name}: ${(error.code || "")} ${error.message.split("\n")[0].slice(0, 200)} [at ${url}]`);
    }
  };

  try {
    await scenario("wikipedia: search box, article, table, links", async (notes) => {
      let view = await open("https://en.wikipedia.org/wiki/Main_Page");
      const box = ref(view.page, (l) => /searchbox|combobox|textbox/.test(l) && /search/i.test(l), "search box");
      await call("act.fill", { ref: box, text: "Alan Turing" });
      const pressed = await call("act.press", { ref: box, key: "Enter" });
      notes.push(`after Enter: ${pressed.url || ""}`);
      await call("wait.load");
      const status = await call("tabs.status");
      check(/Alan_Turing/.test(status.url), `search did not open the article: ${status.url}`);
      const article = await call("extract.article");
      check(article.words > 2000, `article too short: ${article.words} words`);
      notes.push(`article ${article.words} words, title ${article.title}`);
      const tables = await call("observe.tables");
      notes.push(`tables: ${JSON.stringify(tables).length} chars`);
      const links = await call("observe.links");
      notes.push(`observe.links: ${JSON.stringify(links).length} chars`);
      const bletchley = await call("locate.link", { text: "Bletchley Park" });
      check(bletchley.count > 0, "no Bletchley Park link");
      view = await call("observe.page");
      notes.push(`outline ${view.page.length} chars, ${view.page.split("\n").length} lines`);
    });

    await scenario("hacker news: read, next page by More link", async (notes) => {
      const view = await open("https://news.ycombinator.com/");
      const stories = view.page.split("\n").filter((l) => /\blink\b/.test(l)).length;
      notes.push(`${stories} link lines`);
      const next = await call("collect.next_page");
      notes.push(`collect.next_page: ${JSON.stringify(next).slice(0, 200)}`);
      const more = next.found ? next.ref : ref(view.page, (l) => /link "More"/.test(l), "More link");
      await call("act.click", { ref: more });
      await call("wait.load");
      check(/[?&]p=2/.test((await call("tabs.status")).url), "More did not open page 2");
    });

    await scenario("books to scrape: listing, record extraction, pagination", async (notes) => {
      const view = await open("https://books.toscrape.com/");
      const links = await call("observe.links");
      const books = (links.links || links.items || []).length;
      notes.push(`links: ${books}`);
      const next = await call("collect.next_page");
      check(next.found, `no next page found: ${JSON.stringify(next)}`);
      await call("act.click", { ref: next.ref });
      await call("wait.load");
      check(/page-2/.test((await call("tabs.status")).url), "next did not open page 2");
      const book = await call("locate.link", { text: "A Light in the" });
      notes.push(`locate.link on page 2: ${book.count}`);
      const page1 = await open("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html");
      const table = await call("observe.tables");
      check(JSON.stringify(table).includes("UPC"), "product table not read");
      notes.push(`product outline ${page1.page.length} chars`);
    });

    await scenario("httpbin form: fill text, radio, checkboxes, textarea, submit, verify echo", async (notes) => {
      const view = await open("https://httpbin.org/forms/post");
      await call("act.fill", { ref: ref(view.page, /Customer name/i), text: "Ada Lovelace" });
      await call("act.fill", { ref: ref(view.page, /Telephone/i), text: "+44 20 7946 0000" });
      await call("act.fill", { ref: ref(view.page, /E-mail/i), text: "ada@example.com" });
      await call("act.check", { ref: ref(view.page, (l) => /radio/.test(l) && /Medium/.test(l), "Medium radio") });
      await call("act.check", { ref: ref(view.page, (l) => /checkbox/.test(l) && /Bacon/.test(l), "Bacon") });
      await call("act.check", { ref: ref(view.page, (l) => /checkbox/.test(l) && /Onion/.test(l), "Onion") });
      await call("act.fill", { ref: ref(view.page, /Delivery instructions/i), text: "Ring twice.\nSecond line." });
      const time = lineOf(view.page, (l) => /Preferred delivery time/i.test(l));
      notes.push(`time field: ${time}`);
      const sent = await call("form.submit", { ref: ref(view.page, /Customer name/i) });
      notes.push(`submitted via ${sent.submitted}`);
      await call("wait.load");
      const text = (await call("observe.text", { max: 4000 })).text;
      for (const want of ['"custname": "Ada Lovelace"', '"size": "medium"', '"bacon"', '"onion"', '"custemail": "ada@example.com"', "Ring twice."]) {
        check(text.includes(want), `echo lacks ${want}:\n${text.slice(0, 800)}`);
      }
    });

    await scenario("the-internet: dropdown, checkboxes", async () => {
      let view = await open("https://the-internet.herokuapp.com/dropdown");
      await call("act.select_option", { ref: ref(view.page, /combobox|listbox/), value: "Option 2" });
      check(/Option 2/.test((await call("observe.page")).page), "dropdown did not change");
      view = await open("https://the-internet.herokuapp.com/checkboxes");
      const boxes = view.page.split("\n").filter((l) => /checkbox/.test(l));
      check(boxes.length === 2, `expected 2 checkboxes, saw ${boxes.length}`);
      const first = Number(boxes[0].match(/^\[(\d+)\]/)[1]);
      await call("act.check", { ref: first });
      const after = (await call("observe.page")).page.split("\n").filter((l) => /checkbox/.test(l));
      check(after.every((l) => / checked/.test(l)), `not both checked: ${after.join(" | ")}`);
    });

    await scenario("the-internet: dynamic loading (wait for text)", async () => {
      const view = await open("https://the-internet.herokuapp.com/dynamic_loading/2");
      await call("act.click", { ref: ref(view.page, /button "Start"/) });
      await call("wait.text", { text: "Hello World!", timeout: 15_000 });
    });

    await scenario("the-internet: JavaScript alert, confirm, prompt", async (notes) => {
      const view = await open("https://the-internet.herokuapp.com/javascript_alerts");
      const alert = await call("act.click", { ref: ref(view.page, /Click for JS Alert/) });
      notes.push(`alert click: ${JSON.stringify(alert.dialog || alert).slice(0, 160)}`);
      await call("dialog.accept");
      let page = (await call("observe.text")).text;
      check(/You successfully clicked an alert/.test(page), "alert result missing");
      await call("act.click", { ref: ref(view.page, /Click for JS Confirm/) });
      await call("dialog.dismiss");
      page = (await call("observe.text")).text;
      check(/You clicked: Cancel/.test(page), "confirm dismiss result missing");
      await call("act.click", { ref: ref(view.page, /Click for JS Prompt/) });
      await call("dialog.accept", { text: "agentgo" });
      page = (await call("observe.text")).text;
      check(/You entered: agentgo/.test(page), "prompt text missing");
    });

    await scenario("the-internet: hover reveals caption", async () => {
      await open("https://the-internet.herokuapp.com/hovers");
      // Plain images are not in the outline; an agent locates them.
      const images = await call("locate.css", { selector: ".figure img" });
      check(images.count >= 3, `figures not found: ${images.count}`);
      await call("act.hover", { ref: images.matches[0].ref });
      check(/name: user1/.test((await call("observe.text")).text), "hover caption not visible");
    });

    await scenario("the-internet: key presses", async () => {
      const view = await open("https://the-internet.herokuapp.com/key_presses");
      const box = ref(view.page, /textbox/);
      await call("act.click", { ref: box });
      for (const key of ["Shift", "Tab", "Escape", "A"]) {
        await call("act.press", { ref: box, key });
        const said = (await call("observe.text")).text.match(/You entered: (\S+)/);
        check(said && said[1].toUpperCase() === key.toUpperCase(), `${key}: page says ${said && said[1]}`);
      }
    });

    await scenario("the-internet: iframe editor (same-origin frame)", async (notes) => {
      const view = await open("https://the-internet.herokuapp.com/iframe");
      notes.push(`outline lines with frame content: ${view.page.split("\n").filter((l) => /textbox|paragraph|Your content goes here/.test(l)).join(" | ").slice(0, 300)}`);
      const frames = await call("observe.iframe_content");
      notes.push(`frames: ${JSON.stringify(frames).slice(0, 300)}`);
    });

    await scenario("the-internet: HTML drag and drop", async () => {
      const view = await open("https://the-internet.herokuapp.com/drag_and_drop");
      const a = ref(view.page, (l) => /"A"/.test(l) || /\bA\b/.test(l.split(" ").slice(1).join(" ")), "box A");
      const b = ref(view.page, (l) => /"B"/.test(l), "box B");
      const dragged = await call("act.drag", { ref: a, toRef: b });
      const boxes = (await call("observe.page")).page.split("\n").filter((l) => /draggable/.test(l)).map((l) => l.replace(/^\[\d+\] /, ""));
      check(boxes.join() === 'draggable "B",draggable "A"', `columns not swapped (${dragged.dragged}): ${boxes.join()}`);
    });

    await scenario("github trending: read repository list", async (notes) => {
      const view = await open("https://github.com/trending");
      const repos = view.page.split("\n").filter((l) => /link "[\w.-]+ \/ [\w.-]+"/.test(l));
      notes.push(`${repos.length} repository links; first: ${repos[0] || "-"}`);
      check(repos.length >= 5, "fewer than 5 repositories found");
    });

    await scenario("google: search as an agent sees it", async (notes) => {
      const view = await call("navigate.url", { query: "AgentGo browser Chromium", timeout: 30_000 });
      const status = await call("tabs.status");
      notes.push(`landed on ${status.url.slice(0, 120)}`);
      const signals = await call("handoff.detect_captcha");
      notes.push(`captcha detected: ${signals.detected}; handoff: ${JSON.stringify(view.handoff || null)}`);
      const results = await call("workflow.search_extract", { query: "Rust programming language", limit: 5 }).catch((e) => ({ error: `${e.code} ${e.message}` }));
      notes.push(`search_extract: ${JSON.stringify(results).slice(0, 400)}`);
      check(!results.error && results.results && results.results.length > 0, `no results: ${JSON.stringify(results).slice(0, 300)}`);
    });

    await scenario("screenshot of a real page", async (notes) => {
      await open("https://www.rust-lang.org/");
      const shot = await call("observe.screenshot");
      notes.push(`keys: ${Object.keys(shot).join(",")}`);
      check(shot.path || shot.data, "no screenshot");
    });
  } finally {
    fs.writeFileSync(path.join(repo, "target", "real-web-report.json"), JSON.stringify(report, null, 2));
    const failed = report.filter((r) => !r.ok);
    console.log(`\n${report.length - failed.length} of ${report.length} real-web scenarios passed`);
    const code = await browser.close();
    process.exitCode = failed.length || code !== 0 ? 1 : 0;
  }
})();
