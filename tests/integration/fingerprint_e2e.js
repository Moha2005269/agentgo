// Fingerprint protection: with an agent connected every site sees the same
// unique fingerprint; with no agent each site gets its own, and neither
// matches the other. The hooks must read as native code and must also cover
// a blank same-origin iframe (a classic way around canvas protection).
const assert = require("node:assert/strict");
const http = require("node:http");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { launch, reservePort, delay } = require("./harness");

const page = `<title>fp</title><canvas id=c width=200 height=60></canvas><script>
const draw=(doc)=>{const k=doc.createElement('canvas');k.width=200;k.height=60;const c=k.getContext('2d');
for(let i=0;i<40;i++){c.fillStyle='rgb('+(i*6)+','+(255-i*5)+','+(i*3)+')';c.fillRect(i*5,(i*7)%50,30,12);}
c.font='16px serif';c.fillText('AgentGo fingerprint',4,40);return k};
const hash=(k)=>{const c=k.getContext('2d');return k.toDataURL().length+'.'+[...c.getImageData(0,0,200,60).data].reduce((a,b,i)=>(a*31+b+i)>>>0,7)};
const main=hash(draw(document));
const f=document.createElement('iframe');document.body.appendChild(f);
const inFrame=hash(draw(f.contentDocument));
const ts=Function.prototype.toString, fn=HTMLCanvasElement.prototype.toDataURL;
const native=ts.call(fn)==='function toDataURL() { [native code] }'&&ts.call(ts)==='function toString() { [native code] }'
  &&fn.name==='toDataURL'&&!('prototype' in fn)&&Object.getOwnPropertyNames(fn).indexOf('toString')<0
  &&ts.call(CanvasRenderingContext2D.prototype.getImageData).includes('[native code]');
document.title=location.hostname+':'+main+':'+inFrame+':'+(native?'native':'tampered');
</script>`;

(async () => {
  const site = http.createServer((_, response) => response.writeHead(200, { "Content-Type": "text/html" }).end(page));
  await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
  const n = site.address().port;
  const port = await reservePort();
  const dataRoot = fs.mkdtempSync(path.join(os.tmpdir(), "agentgo-fp-"));
  let browser = launch({ args: ["--url", "about:blank"], debugPort: port, dataRoot });
  const hashOf = async (host) => {
    await browser.call("navigate.url", { url: `http://${host}:${n}/`, observe: "none" });
    await browser.call("wait.load", {});
    const title = (await browser.call("observe.text", {})).text || "";
    const info = await browser.call("tabs.list", {});
    const tab = (info.tabs || []).find((t) => (t.url || "").includes(`${host}:${n}`));
    return (tab && tab.title) || title;
  };
  try {
    await browser.call("agent.connect", { name: "fp-test" });
    await delay(1500);
    const a = (await hashOf("127.0.0.1")).split(":").slice(1).join(":");
    const b = (await hashOf("localhost")).split(":").slice(1).join(":");
    assert.ok(a.length > 3, `page computed a fingerprint (${a})`);
    const [mainA, frameA, nativeA] = a.split(":");
    assert.equal(nativeA, "native", "the protection hooks read as native functions");
    assert.equal(frameA, mainA, "a blank iframe gets the same protection as its page");
    assert.equal(a, b, "an agent sees the same fingerprint on every site");
    const status = await browser.call("agent.status", {});
    for (const agent of status.agents || []) {
      await browser.call("agent.disconnect", { name: agent.agent }).catch(() => {});
    }
    assert.equal(((await browser.call("agent.status", {})).agents || []).length, 0, "no agent connected");
    await delay(1500);
    const c = (await hashOf("127.0.0.1")).split(":").slice(1).join(":");
    const d = (await hashOf("localhost")).split(":").slice(1).join(":");
    assert.notEqual(c, d, "with no agent each site gets its own fingerprint");
    assert.notEqual(c, a, "the person's fingerprint never matches the agent's");
    assert.notEqual(d, a, "the person's fingerprint never matches the agent's");
    assert.equal(c.split(":")[2], "native", "browsing mode hooks read as native too");
    assert.equal(c.split(":")[1], c.split(":")[0], "browsing mode covers blank iframes too");
    // The agent keeps its own fingerprint across restarts of the browser.
    await browser.close();
    browser = launch({ args: ["--url", "about:blank"], debugPort: await reservePort(), dataRoot });
    await browser.call("agent.connect", { name: "fp-test" });
    await delay(1500);
    const again = (await hashOf("127.0.0.1")).split(":").slice(1).join(":");
    assert.equal(again, a, "the agent's fingerprint survives a restart");
    console.log("fingerprint_e2e ok");
  } finally {
    site.close();
    await browser.close();
    fs.rmSync(dataRoot, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
