// Account Center: an agent may not act as the person on a site where they are
// signed in until the person links that account, after accepting the
// full-control warning. Linking, unlinking and adding sites are operator-only.
// A local fixture site stands in for a provider (its session is one cookie).
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const http = require("node:http");

// A stand-in OAuth provider with an API: checks PKCE like a real one, hands
// out short-lived tokens and records what reaches it.
function mockProvider() {
  const seen = { challenges: new Map(), calls: [], tokenBodies: [], issued: 0, hold: false };
  const json = { "Content-Type": "application/json" };
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url, "http://mock");
    let body = "";
    for await (const chunk of request) body += chunk;
    const send = (status, value) => response.writeHead(status, json).end(JSON.stringify(value));
    if (url.pathname === "/authorize") {
      seen.authorize = Object.fromEntries(url.searchParams);
      const code = "code-" + crypto.randomUUID();
      seen.challenges.set(code, url.searchParams.get("code_challenge"));
      seen.lastCode = code;
      if (seen.hold) return response.writeHead(200, { "Content-Type": "text/html" }).end("<title>Consent</title><h1>Consent</h1>");
      const back = `${url.searchParams.get("redirect_uri")}?code=${code}&state=${encodeURIComponent(url.searchParams.get("state"))}`;
      return response.writeHead(302, { Location: back }).end();
    }
    if (url.pathname === "/token") {
      const form = new URLSearchParams(body);
      seen.tokenBodies.push(Object.fromEntries(form));
      if (form.get("grant_type") === "authorization_code") {
        const want = seen.challenges.get(form.get("code"));
        const got = crypto.createHash("sha256").update(form.get("code_verifier") || "").digest("base64url");
        if (!want || want !== got) return send(400, { error: "invalid_grant" });
        seen.challenges.delete(form.get("code"));
      } else if (form.get("refresh_token") !== "refresh-1") return send(400, { error: "invalid_grant" });
      seen.issued += 1;
      return send(200, { access_token: `access-${seen.issued}`, refresh_token: "refresh-1", expires_in: 1, token_type: "Bearer" });
    }
    if (url.pathname.startsWith("/api/")) {
      seen.calls.push({ method: request.method, path: url.pathname + url.search, auth: request.headers.authorization, cookie: request.headers.cookie, body });
      if (request.headers.authorization !== `Bearer access-${seen.issued}`) return send(401, { error: "expired" });
      if (url.pathname === "/api/away") return response.writeHead(302, { Location: `http://127.0.0.1:${server.address().port}/api/stolen` }).end();
      return send(200, { items: ["a", "b"], echo: body ? JSON.parse(body) : null });
    }
    send(404, {});
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve({ seen, base: `http://127.0.0.1:${server.address().port}`, close: () => server.close() })));
}
const { launch, operator, cdpEval, serve, reservePort, rejects, refOf, delay } = require("./harness");

async function until(read, done, what) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const value = await read().catch(() => undefined);
    if (done(value)) return value;
    await delay(120);
  }
  throw new Error(`timed out waiting for ${what}`);
}

(async () => {
  const port = await reservePort();
  const site = await serve();
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full", "--allow-evaluate"], debugPort: port });
  const home = site.url("account.html");
  let failed = false;
  try {
    // The built-in list, readable by agents, with the warning the person sees.
    const builtin = (await browser.call("accounts.list")).accounts;
    assert.deepEqual(builtin.map((a) => a.account), ["google", "github", "yandex", "linkedin"]);
    assert(builtin.every((a) => a.linked === false && a.warning.includes("100% control")));
    assert.deepEqual(builtin.find((a) => a.account === "google").connectors, ["drive", "gmail", "photos", "calendar"]);
    assert.equal(JSON.stringify(builtin).includes("token"), false);

    // Only the person changes the list.
    for (const [method, params] of [
      ["accounts.add", { name: "Jobs", signInUrl: home, cookie: "sid" }],
      ["accounts.link", { account: "google", acknowledge: true }],
      ["accounts.unlink", { account: "google" }],
    ]) await rejects(browser.call(method, params), "policy.denied");

    // The person adds a site. Unsafe addresses are refused.
    await rejects(operator(port, "accounts.add", { name: "Evil", signInUrl: "http://evil.example/login", cookie: "sid" }), "invalid.params");
    await rejects(operator(port, "accounts.add", { name: "Dup", signInUrl: "https://mail.google.com/", cookie: "SID" }), "invalid.params");
    const added = (await operator(port, "accounts.add", { name: "Jobs", signInUrl: home, cookie: "sid" })).accounts.at(-1);
    assert.equal(added.custom, true);
    assert.equal(added.linked, false);
    const account = added.account;

    // Signed out, the site is an ordinary page for the agent.
    let view = await browser.call("navigate.url", { url: home });
    assert.match((await browser.call("observe.text")).text, /signed out/);

    // Agents may use a signed-in account unless the person blocks it.
    await browser.call("act.click", { ref: refOf(view.page, "Sign in") });
    assert.match((await browser.call("observe.text")).text, /signed in as ada/);
    await rejects(browser.call("accounts.block", { account }), "policy.denied");
    assert.equal((await operator(port, "accounts.block", { account })).blocked, true);
    assert.equal((await browser.call("accounts.list")).accounts.at(-1).blocked, true);

    // Blocked: every way to read or act on the signed-in site is refused.
    const denied = await rejects(browser.call("observe.page"), "policy.denied");
    assert.match(denied.message, /Jobs is blocked for agents/);
    assert.deepEqual(denied.data, { account, blocked: true });
    for (const [method, params] of [
      ["observe.text", {}],
      ["observe.screenshot", {}],
      ["runtime.evaluate", { expression: "document.cookie" }],
      ["act.click", { x: 40, y: 150 }],
      ["keyboard.type", { text: "x" }],
      ["storage.cookies_list", {}],
      ["navigate.url", { url: home }],
    ]) await rejects(browser.call(method, params), "policy.denied");
    const batch = await browser.call("batch", { steps: [{ method: "observe.text" }] }).catch((error) => error);
    assert.equal(JSON.stringify(batch.steps ?? batch.data ?? batch.message).includes("signed in as"), false);
    // It can still leave.
    await browser.call("navigate.url", { url: "about:blank" });
    assert.equal((await operator(port, "accounts.unblock", { account })).blocked, false);

    // Linking shows the warning first and needs an explicit yes.
    await rejects(operator(port, "accounts.link", { account }), "approval.required");
    assert.equal((await browser.call("accounts.list")).accounts.at(-1).linked, false);
    const linked = await operator(port, "accounts.link", { account, acknowledge: true, openSignIn: false });
    assert.equal(linked.linked, true);
    const listed = (await browser.call("accounts.list")).accounts.at(-1);
    assert.equal(listed.linked, true);
    assert(listed.linkedAt > 1_700_000_000);

    // Now the agent uses the account as its own.
    view = await browser.call("navigate.url", { url: home });
    assert.match((await browser.call("observe.text")).text, /signed in as ada/);
    await browser.call("act.click", { ref: refOf(view.page, "Apply now") });
    assert.match((await browser.call("observe.text")).text, /application sent/);

    // Unlinking takes the access back and signs the account out here.
    const unlinked = await operator(port, "accounts.unlink", { account });
    assert.equal(unlinked.wasLinked, true);
    await browser.call("navigate.url", { url: home });
    assert.match((await browser.call("observe.text")).text, /signed out/);
    assert.equal((await operator(port, "accounts.unlink", { account, forget: true })).linked, false);
    assert.equal((await browser.call("accounts.list")).accounts.length, 4);

    // The same through the Account Center panel, as the person does it.
    const tools = (js) => cdpEval(port, "tools.html", js);
    await cdpEval(port, "chrome.html", "document.querySelector('#openAccounts').click(); true");
    await until(() => tools("document.querySelectorAll('[data-acc-link]').length"), (n) => n === 4, "account rows");
    await tools("document.querySelector('[data-acc-block=\"linkedin\"]').click(); true");
    await until(() => browser.call("accounts.list"), (list) => list?.accounts.find((a) => a.account === "linkedin").blocked, "block from the panel");
    await until(() => tools("document.querySelector('[data-acc-block=\"linkedin\"]')?.textContent"), (label) => label === "Unblock", "unblock button");
    await tools("document.querySelector('[data-acc-block=\"linkedin\"]').click(); true");
    await until(() => browser.call("accounts.list"), (list) => list?.accounts.find((a) => a.account === "linkedin").blocked === false, "unblock from the panel");
    await tools("document.querySelector('[data-acc-link=\"linkedin\"]').click(); true");
    assert.match(await tools("document.querySelector('.need').textContent"), /100% control[\s\S]*apply for jobs[\s\S]*never sees your password/);
    // Nothing is linked until the warning is confirmed; Cancel backs out.
    assert.equal((await browser.call("accounts.list")).accounts.find((a) => a.account === "linkedin").linked, false);
    await tools("document.querySelector('[data-acc-cancel]').click(); true");
    assert.equal(await tools("document.querySelectorAll('.need').length"), 0);
    await tools(`const f = document.querySelector('#accAdd'); f.elements.name.value = 'Jobs'; f.elements.signInUrl.value = ${JSON.stringify(home)}; f.elements.cookie.value = 'sid'; f.requestSubmit(); true`);
    await until(() => tools("document.querySelectorAll('[data-acc-link]').length"), (n) => n === 5, "added site row");
    await tools(`document.querySelector('[data-acc-link="127-0-0-1-${new URL(home).port}"]').click(); true`);
    await tools("document.querySelector('[data-acc-confirm]').click(); true");
    const signIn = await until(() => operator(port, "tabs.list"), (t) => t?.tabs.some((tab) => tab.url === home && tab.active), "sign-in tab");
    assert(signIn);
    assert.equal((await browser.call("accounts.list")).accounts.at(-1).linked, true);

    // Saved sign-ins: the person stores one, the browser types it for the
    // agent, and the password never reaches the agent.
    const secret = "hunter2-vault";
    const loginPage = site.url("login.html");
    const origin = new URL(loginPage).origin;
    await rejects(browser.call("credentials.save", { origin, username: "ada@example.com", password: secret }), "policy.denied");
    await rejects(operator(port, "credentials.save", { origin: "http://example.com", username: "a", password: "b" }), "invalid.params");
    await operator(port, "credentials.save", { origin: loginPage, username: "ada@example.com", password: secret });
    const saved = await browser.call("credentials.list");
    assert.deepEqual(saved.credentials, [{ origin, username: "ada@example.com" }]);
    const form = await browser.call("navigate.url", { url: loginPage });
    const filled = await browser.call("credentials.fill");
    assert.deepEqual(filled.filled, ["username", "password"]);
    const seen = JSON.stringify([saved, filled, await browser.call("observe.page"), await browser.call("observe.forms"), await browser.call("audit.list").catch(() => ({}))]);
    assert.equal(seen.includes(secret), false, "the password reached the agent");
    await browser.call("act.click", { ref: refOf(form.page, 'button "Sign in"') });
    assert.match((await browser.call("observe.text")).text, /welcome ada@example.com/);
    // Only on the exact origin it was saved for.
    await browser.call("navigate.url", { url: "about:blank" });
    await rejects(browser.call("credentials.fill"), "policy.denied");
    assert.deepEqual((await operator(port, "credentials.delete", { origin })).credentials, []);

    // After a sign-in the browser offers to save it, also when an agent typed
    // it. Nothing is stored until the person says yes.
    const typed = "typed-by-agent-9";
    const loginForm = await browser.call("navigate.url", { url: loginPage });
    await browser.call("act.fill", { ref: refOf(loginForm.page, "Email"), text: "bob@example.com" });
    await browser.call("act.fill", { ref: refOf(loginForm.page, "Password"), text: typed });
    await browser.call("act.click", { ref: refOf(loginForm.page, 'button "Sign in"') });
    const activeTab = async () => (await operator(port, "tabs.list")).tabs.find((tab) => tab.url === loginPage);
    const offered = await until(activeTab, (tab) => tab?.saveLogin, "offer to save");
    assert.deepEqual(offered.saveLogin, { origin, username: "bob@example.com", update: false });
    assert.equal(JSON.stringify(offered).includes(typed), false);
    assert.deepEqual((await browser.call("credentials.list")).credentials, []);
    await rejects(browser.call("credentials.offer_accept", {}), "policy.denied");
    // The person finds the question on the key button of that tab.
    await operator(port, "tabs.activate", { tabId: offered.tabId });
    await until(() => cdpEval(port, "chrome.html", "document.querySelector('#loginKey').hidden"), (hidden) => hidden === false, "key button");
    await cdpEval(port, "chrome.html", "document.querySelector('#loginKey').click(); true");
    await until(() => tools("document.querySelector('[data-offer=\"accept\"]')?.textContent"), (label) => label === "Save", "save prompt");
    await tools("document.querySelector('[data-offer=\"accept\"]').click(); true");
    await until(() => browser.call("credentials.list"), (list) => list?.credentials.length === 1, "saved sign-in");
    assert.equal((await activeTab()).hasLogin, true);
    // The same sign-in again is not offered twice; "never" stops the question.
    await browser.call("act.click", { ref: refOf(loginForm.page, 'button "Sign in"') });
    await delay(400);
    assert.equal((await activeTab()).saveLogin, null);
    await browser.call("act.fill", { ref: refOf(loginForm.page, "Password"), text: "changed-password-1" });
    await browser.call("act.click", { ref: refOf(loginForm.page, 'button "Sign in"') });
    assert.equal((await until(activeTab, (tab) => tab?.saveLogin, "offer to update")).saveLogin.update, true);
    await operator(port, "credentials.offer_dismiss", { tabId: offered.tabId, never: true });
    await browser.call("act.fill", { ref: refOf(loginForm.page, "Password"), text: "changed-password-2" });
    await browser.call("act.click", { ref: refOf(loginForm.page, 'button "Sign in"') });
    await delay(400);
    assert.equal((await activeTab()).saveLogin, null);
    // One click by the person fills the saved sign-in.
    await browser.call("navigate.url", { url: loginPage });
    await cdpEval(port, "chrome.html", "document.querySelector('#loginKey').click(); true");
    await until(() => browser.call("observe.forms"), (forms) => JSON.stringify(forms).includes("bob@example.com"), "one-click fill");
    await operator(port, "credentials.delete", { origin });

    // ---- API connectors: OAuth sign-in and the token-free gateway ----------
    const mock = await mockProvider();
    const apiAccount = (await operator(port, "accounts.add", {
      name: "Mock API", signInUrl: `${mock.base}/login`, cookie: "m",
      authorizeUrl: `${mock.base}/authorize`, tokenUrl: `${mock.base}/token`, apiBase: `${mock.base}/api`, scopes: "read write",
    })).accounts.at(-1).account;
    const call = (params) => browser.call("connectors.call", { account: apiAccount, connector: "api", ...params });
    await rejects(browser.call("accounts.connect", { account: apiAccount, clientId: "client-1", acknowledge: true }), "policy.denied");
    await rejects(call({ path: "/items" }), "policy.denied");
    await rejects(operator(port, "accounts.connect", { account: apiAccount, clientId: "client-1" }), "approval.required");
    await rejects(operator(port, "accounts.connect", { account: "linkedin", clientId: "x", acknowledge: true }), "invalid.params");
    const apiState = async () => (await browser.call("accounts.list")).accounts.find((a) => a.account === apiAccount).api;

    const started = await operator(port, "accounts.connect", { account: apiAccount, clientId: "client-1", acknowledge: true });
    await until(apiState, (state) => state?.connected, "API sign-in");
    assert.equal(mock.seen.authorize.code_challenge_method, "S256");
    assert.equal(mock.seen.authorize.response_type, "code");
    assert.equal(mock.seen.authorize.redirect_uri, started.redirectUri);
    assert.match(started.redirectUri, /^http:\/\/127\.0\.0\.1:\d+\/callback$/);
    assert.equal(mock.seen.authorize.scope, "read write");
    assert.equal(mock.seen.tokenBodies[0].code_verifier.length, 43);
    assert.equal("client_secret" in mock.seen.tokenBodies[0], false);

    const connectors = await browser.call("connectors.list");
    assert.deepEqual(connectors.connectors.map((c) => [c.account, c.connector]), [[apiAccount, "api"]]);
    const got = await call({ path: "/items?limit=2" });
    assert.equal(got.status, 200);
    assert.deepEqual(got.json.items, ["a", "b"]);
    const posted = await call({ method: "post", path: "/items", body: { title: "from the agent" } });
    assert.deepEqual(posted.json.echo, { title: "from the agent" });
    const last = mock.seen.calls.at(-1);
    assert.equal(last.method, "POST");
    assert.match(last.auth, /^Bearer access-\d+$/);
    assert.equal(last.cookie, undefined, "profile cookies were sent to the API");
    assert(mock.seen.tokenBodies.some((body) => body.grant_type === "refresh_token"), "a stale token was not refreshed");
    // The agent never sees a token, in any answer.
    const exposed = JSON.stringify([started, connectors, got, posted, await browser.call("accounts.list"), await browser.call("audit.list").catch(() => ({}))]);
    assert.equal(/access-\d|refresh-1/.test(exposed), false, "a token reached the agent");
    // A call cannot leave its API, and a redirect is not followed with the token.
    for (const path of ["/../token", "//127.0.0.1/x", "items", "/a\\b"]) await rejects(call({ path }), "invalid.params");
    await rejects(call({ method: "TRACE", path: "/items" }), "invalid.params");
    assert.equal((await call({ path: "/away" })).status, 302);
    assert.equal(mock.seen.calls.some((c) => c.path === "/api/stolen"), false, "the token followed a redirect");

    // A forged answer (wrong state) to a running sign-in connects nothing and
    // uses the attempt up: the real code is worthless afterwards.
    mock.seen.hold = true;
    const issuedBefore = mock.seen.issued;
    const second = await operator(port, "accounts.connect", { account: apiAccount, clientId: "client-1", acknowledge: true });
    await until(async () => mock.seen.lastCode && mock.seen.authorize.redirect_uri === second.redirectUri, Boolean, "second authorize");
    await fetch(`${second.redirectUri}?code=attacker-code&state=attacker-state`);
    await until(apiState, (state) => /does not belong to this sign-in attempt/.test(state?.error || ""), "refused forged answer");
    await fetch(`${second.redirectUri}?code=${mock.seen.lastCode}&state=${mock.seen.authorize.state}`).catch(() => {});
    await delay(600);
    assert.equal(mock.seen.issued, issuedBefore, "a used-up sign-in attempt was accepted");

    // Unlinking deletes the tokens.
    await operator(port, "accounts.unlink", { account: apiAccount });
    await rejects(call({ path: "/items" }), "policy.denied");
    assert.deepEqual((await browser.call("connectors.list")).connectors, []);
    assert.equal((await apiState()).connected, false);
    mock.close();

    console.log(JSON.stringify({ ok: true }));
  } catch (error) {
    console.error(error);
    failed = true;
  } finally {
    const code = await browser.close();
    site.close();
    process.exitCode = failed || code !== 0 ? 1 : 0;
  }
})();
