// Ed25519 signing for AgentGo's update manifest.
//
//   node scripts/dev/sign-update.js keygen            create the key pair (once)
//   node scripts/dev/sign-update.js sign <manifest>   add "signature" to a manifest
//
// The private key lives OUTSIDE the repository, in
// %USERPROFILE%\.agentgo-signing\update-ed25519.pem (override: AGENTGO_UPDATE_KEY).
// Back it up: every release must be signed with the same key, because the
// matching public key is compiled into the browser
// (crates/agentgo-cef/src/runtime/update-key.pub).
"use strict";
const crypto = require("crypto");
const fs = require("fs");
const os = require("os");
const path = require("path");

const keyPath = process.env.AGENTGO_UPDATE_KEY || path.join(os.homedir(), ".agentgo-signing", "update-ed25519.pem");
const pubPath = path.join(__dirname, "..", "..", "crates", "agentgo-cef", "src", "runtime", "update-key.pub");

// Must match `signed_text` in lifecycle.rs.
function signedText(m) {
  return `agentgo-update-v1\n${m.version}\n${m.url}\n${String(m.sha256).toLowerCase()}\n`;
}

const [, , command, file] = process.argv;
if (command === "keygen") {
  if (fs.existsSync(keyPath)) {
    console.error(`refusing to overwrite ${keyPath}`);
    process.exit(1);
  }
  const { privateKey, publicKey } = crypto.generateKeyPairSync("ed25519");
  fs.mkdirSync(path.dirname(keyPath), { recursive: true });
  fs.writeFileSync(keyPath, privateKey.export({ type: "pkcs8", format: "pem" }), { mode: 0o600 });
  const raw = publicKey.export({ type: "spki", format: "der" }).subarray(-32);
  fs.writeFileSync(pubPath, raw.toString("hex") + "\n");
  console.log(`private key: ${keyPath}\npublic key:  ${pubPath} (${raw.toString("hex")})`);
} else if (command === "sign" && file) {
  if (!fs.existsSync(keyPath)) {
    console.error(`no signing key at ${keyPath}; run keygen first`);
    process.exit(2);
  }
  const manifest = JSON.parse(fs.readFileSync(file, "utf8").replace(/^\uFEFF/, ""));
  delete manifest.signature;
  const key = crypto.createPrivateKey(fs.readFileSync(keyPath));
  manifest.signature = crypto.sign(null, Buffer.from(signedText(manifest)), key).toString("hex");
  fs.writeFileSync(file, JSON.stringify(manifest, null, 2) + "\n");
  console.log(`signed ${file}`);
} else {
  console.error("usage: sign-update.js keygen | sign <manifest.json>");
  process.exit(64);
}
