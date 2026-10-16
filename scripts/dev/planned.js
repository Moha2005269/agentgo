// Lists the registry commands that have no integration test yet (= planned).
const fs = require("node:fs");
const path = require("node:path");
const source = fs.readFileSync(path.join(__dirname, "../../crates/agentgo-shell-bridge/src/methods.rs"), "utf8");
const entry = /\(\s*"([a-z_.0-9]+)",\s*"((?:[^"\\]|\\.)*)",\s*(\d+),\s*"([^"]*)",?\s*\)/g;
const planned = [];
let total = 0;
for (let m; (m = entry.exec(source)); ) {
  total += 1;
  if (!m[4]) planned.push([Number(m[3]), m[1], m[2]]);
}
planned.sort((a, b) => a[0] - b[0] || a[1].localeCompare(b[1]));
console.log(`${total} commands, ${planned.length} planned`);
for (const [milestone, name, text] of planned) console.log(`M${milestone} ${name} — ${text.slice(0, 80)}`);
