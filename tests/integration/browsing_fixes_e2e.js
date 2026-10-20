// The browsing fixes of 0.1.5, each against a page that shows the old bug:
//  - navigate.url answers with what load handlers drew, not the empty page;
//  - script-driven icons (a click listener, no text, no pointer cursor) are
//    in the outline, named by their class;
//  - act.fill sets date, time, colour and range inputs with input/change;
//  - act.type types into the element `ref` names, as key presses;
//  - a newsletter "Subscribe" checkbox is not a payment;
//  - a popup's question is in the outline, not only its buttons.
// And those the 0.1.5 agent benchmark found (page two):
//  - a value is shown exactly (a trailing space is part of it);
//  - a sentence keeps the number in its inline element;
//  - an equation is not taken for the label of the field after it;
//  - unnamed radios are "1 of 3"; a field alone in a box is named by the
//    text before the box; an open tree folder is named by its own label;
//  - a list that takes several options gets them in one select;
//  - script links without an address (a calendar's Prev) are controls.
const assert = require("node:assert/strict");
const http = require("node:http");
const { launch } = require("./harness");

const page = `<!doctype html><title>fixes</title>
<label>Start <input id="date" type="date"></label>
<label>At <input id="time" type="time"></label>
<label>Tint <input id="color" type="color"></label>
<label>Level <input id="range" type="range" min="0" max="10"></label>
<label>Name <input id="name"></label>
<label><input id="news" type="checkbox"> Subscribe to our newsletter</label>
<div id="later"></div>
<pre id="log"></pre>
<input id="keys" aria-label="terminal" style="position:absolute;left:-9999px">
<script>
const log = (k, v) => { document.getElementById('log').textContent += k + '=' + v + ';'; };
for (const id of ['date', 'time', 'color', 'range'])
  document.getElementById(id).addEventListener('change', e => log(id, e.target.value));
document.getElementById('keys').addEventListener('keydown', e => { if (e.key.length === 1) log('key', e.key); });
window.addEventListener('load', () => setTimeout(() => {
  const later = document.getElementById('later');
  for (const name of ['star', 'trash']) {
    const icon = document.createElement('span');
    icon.className = 'icon ' + name;
    icon.style.cssText = 'display:inline-block;width:14px;height:14px;background:#888';
    icon.addEventListener('click', () => log('icon', name));
    later.append(icon);
  }
  const popup = document.createElement('div');
  popup.id = 'popup';
  popup.style.cssText = 'position:absolute;top:10px;left:200px;background:#fff';
  popup.innerHTML = '<p>Your session is about to expire.</p><button>OK</button> <button>Cancel</button>';
  document.body.append(popup);
}, 80));
</script>`;

const two = `<!doctype html><title>two</title>
<textarea id="src">Commodo lacus </textarea>
<p id="say">The number is lower than <span>5</span>.</p>
<div>5 - x = 79</div><label>x = <input id="x"></label>
<div><input type="radio" name="pick" value="1"><input type="radio" name="pick" value="2"><input type="radio" name="pick" value="3"></div>
<div class="head">Departure Date</div><div class="box"><input id="when" readonly></div>
<ul id="tree"><li><span class="folder">Keli</span><ul><li><span class="file">Briana</span></li></ul></li></ul>
<select id="many" multiple size="4"><option>Namibia</option><option>Peru</option><option>Puerto Rico</option><option>Chad</option></select>
<div class="cal"><a class="prev" title="Prev" data-handler="prev">Prev</a> <span>December</span> <span>2016</span> <a href="#">1</a> <a href="#">2</a></div>
<pre id="log2"></pre>
<script>
for (const li of document.querySelectorAll('#tree li')) li.addEventListener('click', e => { document.getElementById('log2').textContent += 'tree=' + li.firstElementChild.textContent + ';'; e.stopPropagation(); });
document.querySelector('.cal').addEventListener('click', e => { if (e.target.closest('.prev')) document.getElementById('log2').textContent += 'prev;'; });
document.getElementById('many').addEventListener('change', e => document.getElementById('log2').textContent += 'many=' + [...e.target.selectedOptions].map(o => o.text).join('+') + ';');
</script>`;

const refOf = (text, pattern) => {
  const line = text.split("\n").find((l) => pattern.test(l));
  assert.ok(line, `no line matches ${pattern} in:\n${text}`);
  return Number(line.match(/^\[(\d+)\]/)[1]);
};

(async () => {
  const site = http.createServer((request, response) => response.writeHead(200, { "Content-Type": "text/html" }).end(request.url === "/two" ? two : page));
  await new Promise((resolve) => site.listen(0, "127.0.0.1", resolve));
  const browser = launch({ args: ["--url", "about:blank", "--mode", "full"] });
  try {
    const open = await browser.call("navigate.url", { url: `http://127.0.0.1:${site.address().port}/` });
    const text = open.page || "";
    assert.match(text, /\(icon: star\)/, `open shows the icons drawn after load:\n${text}`);
    assert.match(text, /\(icon: trash\)/);
    assert.match(text, /dialog "Your session is about to expire\."/, `the popup's question is in the outline:\n${text}`);

    for (const [label, value, want] of [["Start", "09/20/2019", "2019-09-20"], ["At", "1:45 pm", "13:45"], ["Tint", "#369", "#336699"], ["Level", "7", "7"]]) {
      const ref = refOf(text, new RegExp(`"${label}"`));
      await browser.call("act.fill", { ref, text: value });
      const got = await browser.call("runtime.get_value", { ref });
      assert.equal(got.value, want, `${label} holds ${want}`);
    }

    await browser.call("act.click", { ref: refOf(text, /\(icon: trash\)/) });
    const name = refOf(text, /"Name"/);
    await browser.call("act.fill", { ref: name, text: "Ada" });
    await browser.call("act.type", { ref: refOf(text, /"terminal"/), text: "ls" });
    const after = await browser.call("runtime.get_value", { ref: name });
    assert.equal(after.value, "Ada", "typing into the terminal leaves the Name field alone");

    const checked = await browser.call("act.check", { ref: refOf(text, /Subscribe/) });
    assert.equal(checked.checked, true, "a newsletter checkbox is ticked without a payment approval");

    const log = (await browser.call("observe.text", {})).text || "";
    for (const part of ["date=2019-09-20", "time=13:45", "color=#336699", "range=7", "icon=trash", "key=l", "key=s"]) {
      assert.ok(log.includes(part), `the page saw ${part}: ${log}`);
    }

    const second = await browser.call("navigate.url", { url: `http://127.0.0.1:${site.address().port}/two` });
    const outline = second.page || "";
    assert.match(outline, /value="Commodo lacus "/, `values are exact:
${outline}`);
    assert.match(outline, /· The number is lower than 5\./, `inline text stays in its sentence:
${outline}`);
    assert.match(outline, /· 5 - x = 79/, `the equation is kept:
${outline}`);
    for (const n of [1, 2, 3]) assert.match(outline, new RegExp(`radio "${n} of 3"`), outline);
    assert.match(outline, /textbox "Departure Date" readonly/, outline);
    assert.equal(outline.split(String.fromCharCode(10)).filter((l) => /"Keli"/.test(l)).length, 1, `one line per tree node:
${outline}`);
    assert.match(outline, /listbox "" value="" multiple options="Namibia \| Peru \| Puerto Rico \| Chad"/, outline);
    await browser.call("act.select_option", { ref: refOf(outline, /^\[\d+\] listbox/), values: ["Namibia", "Puerto Rico"] });
    await browser.call("act.click", { ref: refOf(outline, /"Keli"/) });
    await browser.call("act.click", { ref: refOf(outline, /"Prev"/) });
    const log2 = (await browser.call("observe.text", {})).text || "";
    for (const part of ["many=Namibia+Puerto Rico", "tree=Keli", "prev"]) assert.ok(log2.includes(part), `page two saw ${part}: ${log2}`);
    console.log("browsing_fixes_e2e ok");
  } finally {
    site.close();
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
