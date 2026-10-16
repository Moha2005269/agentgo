// Every extra command of the tool register (scripts/dev/tools-spec.js) is
// called for real against a fixture page and must return what its line in the
// spec says. A name cannot be listed as implemented without passing here.
// Names that are in the register but not built must say so.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { launch, serve, repo } = require("./harness");
const spec = require("../../scripts/dev/tools-spec");

(async () => {
  const site = await serve();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"] });
  const failures = [];
  let ran = 0;
  try {
    // Speed: the visuals are tested elsewhere.
    await browser.call("settings.set", { settings: { agentAnimations: false } });
    const home = site.url("tools.html");
    const open = async () => { await browser.call("navigate.url", { url: home, observe: "none" }); };
    await open();
    const refs = {};
    const refOf = async (id) => {
      if (refs[id]) return refs[id];
      let found = (await browser.call("locate.css", { selector: `#${id}` })).matches[0];
      if (!found) {
        // A case before (reload, then stop) can leave a half-loaded copy of
        // the fixture at the same address: load it again once.
        await open();
        for (const key of Object.keys(refs)) delete refs[key];
        found = (await browser.call("locate.css", { selector: `#${id}` })).matches[0];
      }
      if (!found) throw Object.assign(new Error(`fixture element #${id} not found`), { code: "fixture.missing" });
      return (refs[id] = found.ref);
    };
    const fill = async (value) => {
      if (typeof value === "string" && value.startsWith("REF.")) return refOf(value.slice(4));
      if (value === "SERP") return site.url("serp.html");
      if (Array.isArray(value)) return Promise.all(value.map(fill));
      if (value && typeof value === "object") return Object.fromEntries(await Promise.all(Object.entries(value).map(async ([k, v]) => [k, await fill(v)])));
      return value;
    };
    const check = async (name, params, expect) => {
      ran += 1;
      // Each case starts from the fixture page, whatever the last one did.
      if ((await browser.call("tabs.status")).url !== home) {
        await open();
        for (const key of Object.keys(refs)) delete refs[key];
      }
      const outcome = await fill(params).then((p) => browser.call(name, p, 20_000)).then((r) => ({ r }), (error) => ({ code: error.code, message: error.message }));
      const accepted = expect.split("|").some((option) => {
        if (option.startsWith("!")) return outcome.code === option.slice(1);
        if (outcome.code) return false;
        try { return Boolean(new Function("r", `return (${option});`)(outcome.r)); } catch (_) { return false; }
      });
      if (!accepted) failures.push(`${name}: expected ${expect}, got ${outcome.code ? `${outcome.code} (${outcome.message.split("\n")[0].slice(0, 120)})` : JSON.stringify(outcome.r).slice(0, 160)}`);
    };
    for (const [name, , , , params, expect] of spec.aliases) await check(name, params, expect);
    await open();
    for (const key of Object.keys(refs)) delete refs[key];
    for (const [name, , params, expect] of spec.page) await check(name, params, expect);
    for (const [name, , , , params, expect] of spec.cdp) await check(name, params, expect);
    // Names this browser cannot provide say why.
    for (const [name, reason] of spec.unsupported) {
      const error = await browser.call(name, {}).catch((e) => e);
      if (error.code !== "capability.unsupported" || !error.message.includes(reason)) failures.push(`${name}: expected capability.unsupported with its reason, got ${error.code}`);
    }

    // A case above opened a DevTools window; close it.
    await browser.call("debug.devtools_close").catch(() => {});

    // A page tool reports a missing element and a missing parameter plainly.
    const missing = await browser.call("runtime.get_text", { selector: "#nope" }).catch((error) => error.code);
    assert.equal(missing, "target.stale");
    assert.equal(await browser.call("locate.css", {}).catch((error) => error.code), "invalid.params");
    // Payment fields are only filled through act.fill, which asks the person.
    assert.equal(await browser.call("form.datetime", { selector: "#expiry", value: "2027-01" }).catch((error) => error.code), "target.blocked");
    // References from a locate command are ordinary references.
    const go = (await browser.call("locate.test_id", { id: "go-button" })).matches[0].ref;
    await browser.call("act.click", { ref: go });

    // Found on a real shop: a link without href that is a button must be in
    // the outline, and a clickable wrapper around one control is not a second
    // target. Line breaks separate lines of text.
    await browser.call("network.online", {}); // a case above took the tab offline
    await browser.call("navigate.url", { url: home, observe: "none" });
    const outline = (await browser.call("observe.page", { max: 200 })).page;
    assert.match(outline, /button "Cart, 2 items"/);
    assert.match(outline, /checkbox "Water the plants"/);
    assert.equal(outline.split("\n").filter((line) => /Sort products|A to Z/.test(line)).length, 1, outline);

    // Names of the register that are not built answer honestly, and every
    // name of the register is known to the browser one way or the other.
    const planned = [...fs.readFileSync(path.join(repo, "crates/agentgo-shell-bridge/src/extra.rs"), "utf8").split("pub const PLANNED")[1].split("pub fn find")[0].matchAll(/"([a-z_.0-9]+)"/g)].map((m) => m[1]);
    // Every name is built now; the check stays for names added later.
    for (const name of planned.length ? [planned[0], planned[Math.floor(planned.length / 2)], planned.at(-1)] : []) {
      const error = await browser.call(name, {}).catch((e) => e);
      assert.equal(error.code, "capability.unsupported", name);
      assert.match(error.message, /not built yet/);
    }
    assert.equal(await browser.call("no.such_command", {}).catch((error) => error.code), "method.notFound");

    if (failures.length) throw new Error(`${failures.length} of ${ran} commands failed:\n${failures.join("\n")}`);
    console.log(JSON.stringify({ ok: true, commands: ran, planned: planned.length }));
  } catch (error) {
    console.error(error.message || error);
    process.exitCode = 1;
  } finally {
    const code = await browser.close();
    site.close();
    if (code !== 0) process.exitCode = 1;
  }
})();
