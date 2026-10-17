// Builds crates/agentgo-adblock/data/threats.txt: the hosts of known malware
// and phishing sites, checked before every page AgentGo opens (the network
// filters alone never stop a top-level page).
//   node scripts/dev/build-threats.js
// Sources (already in data/lists): URLhaus (malware), uBlock Badware risks
// (scams, fake updates, malware), Phishing URL Blocklist (OpenPhish, IPThreat,
// PhishTank via malware-filter). One "<m|p> <host>" per line, sorted.
"use strict";
const fs = require("node:fs");
const path = require("node:path");

const data = path.join(__dirname, "..", "..", "crates", "agentgo-adblock", "data");
const sources = [
  ["m", "urlhaus.txt"],
  ["m", "ublock-badware.txt"],
  ["p", "phishing.txt"],
];
// `||host^`, optionally with options that still mean "the whole site"
// (document, all, or no resource-type restriction).
const rule = /^\|\|([a-z0-9.-]+|\[[0-9a-f:]+\])\^(?:\$(.*))?$/i;
const wholeSite = (options) => {
  if (!options) return true;
  const list = options.split(",").map((o) => o.trim().toLowerCase());
  if (list.some((o) => o.startsWith("domain=") || o.startsWith("~") || o === "badfilter" || o.startsWith("denyallow"))) return false;
  const types = list.filter((o) => !o.startsWith("important") && o !== "third-party" && o !== "3p" && o !== "1p" && o !== "first-party");
  return types.length === 0 || types.some((o) => o === "doc" || o === "document" || o === "all");
};

const hosts = new Map();
for (const [kind, file] of sources) {
  const text = fs.readFileSync(path.join(data, "lists", file), "utf8");
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    const m = rule.exec(line);
    if (!m || !wholeSite(m[2])) continue;
    const host = m[1].toLowerCase().replace(/\.$/, "");
    if (!host.includes(".") && !host.startsWith("[")) continue;
    // Malware wins over phishing when a host is in both.
    if (!hosts.has(host) || kind === "m") hosts.set(host, kind);
  }
}
const lines = [...hosts].sort((a, b) => (a[0] < b[0] ? -1 : 1)).map(([host, kind]) => `${kind} ${host}`);
fs.writeFileSync(path.join(data, "threats.txt"), lines.join("\n") + "\n");
const count = (k) => lines.filter((l) => l[0] === k).length;
console.log(`threats.txt: ${lines.length} hosts (${count("m")} malware, ${count("p")} phishing)`);
