// Adds commands to the registry table (crates/agentgo-shell-bridge/src/methods.rs)
// unless they are already there.
//   node scripts/dev/add-methods.js <file.json>   [[name, description, test], ...]
const fs = require("node:fs");
const path = require("node:path");

const file = path.resolve(__dirname, "../../crates/agentgo-shell-bridge/src/methods.rs");
let source = fs.readFileSync(file, "utf8");
const eol = source.includes("\r\n") ? "\r\n" : "\n";
const entries = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const end = source.indexOf(`${eol}];${eol}`);
let added = "";
let count = 0;
for (const [name, description, test] of entries) {
  if (source.includes(`"${name}",${eol}`)) continue;
  added += `    (${eol}        ${JSON.stringify(name)},${eol}        ${JSON.stringify(description)},${eol}        12,${eol}        ${JSON.stringify(test)},${eol}    ),`;
  added += eol;
  count += 1;
}
source = source.slice(0, end + eol.length) + added + source.slice(end + eol.length);
fs.writeFileSync(file, source);
console.log(`added ${count} commands`);
