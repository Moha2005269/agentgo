// Fingerprint protection: with an agent connected every site sees the same
// unique fingerprint; with no agent each site gets its own.
const assert = require("node:assert/strict");
const http = require("node:http");
const { launch, reservePort, delay } = require("./harness");

const page = `<title>fp</title><canvas id=c width=200 height=60></canvas><script>
const c=document.getElementById('c').getContext('2d');
for(let i=0;i<40;i++){c.fillStyle='rgb('+(i*6)+','+(255-i*5)+','+(i*3)+')';c.fillRect(i*5,(i*7)%50,30,12);}
c.font='16px serif';c.fillText('AgentGo fingerprint',4,40);
document.title=location.hostname+':'+document.getElementById('c').toDataURL().length+':'+[...c.getImageData(0,0,200,60).data].reduce((a,b,i)=>(a*31+b+i)>>>0,7);
</script>`;

(async () => {
  const site = http.createServer((_, response) => response.writeHead(200, { "Content-Type": "text/html" }).end(page));
  await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
  const n = site.address().port;
  const port = await reservePort();
  const browser = launch({ args: ["--url", "about:blank"], debugPort: port });
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
    console.log("fingerprint_e2e ok");
  } finally {
    site.close();
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
