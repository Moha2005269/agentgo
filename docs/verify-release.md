# Verifying a release download

Every file on an AgentGo release (from 0.1.3 on) has a Sigstore Cosign
signature bundle next to it: `AgentGo-Setup.exe` has
`AgentGo-Setup.exe.sigstore.json`, and so on. The repository's
`sign-release` workflow makes them when a release is published, keyless:
the signing certificate names that workflow in this repository, and the
signature is recorded in Sigstore's public transparency log (Rekor).

## Check a file

Install [Cosign](https://docs.sigstore.dev/cosign/system_config/installation/)
(for example `winget install sigstore.cosign`), download the file and its
`.sigstore.json`, then:

```
cosign verify-blob AgentGo-Setup.exe --bundle AgentGo-Setup.exe.sigstore.json --certificate-identity-regexp "^https://github.com/Moha2005269/agentgo/.github/workflows/sign-release.yml@" --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

`Verified OK` means the file is exactly what this repository's workflow
signed. Any change to the file, or a signature from somewhere else, fails.

## What else is checked

- Each file also has a `.sha256` on the release.
- AgentGo's own updater checks a separate Ed25519 release signature and the
  SHA-256 before it installs an update.
- AgentGo Setup checks every installed file against `SHA256SUMS.txt`.

## What this is not

A Cosign signature is not an Authenticode signature. Windows SmartScreen
does not read it, so Windows may still say the publisher is unknown the
first time you run AgentGo Setup. Removing that warning needs a code-signing
certificate, which AgentGo does not have yet.
