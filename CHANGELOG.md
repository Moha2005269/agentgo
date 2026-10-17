# Changelog

## 0.1.0 (2026-10-16)

First public build.

- Browser: tabs, history, bookmarks, favorites bar, downloads, find, zoom, print and print to PDF, session restore, Settings with search, extensions from the Chrome Web Store.
- Agent control: MCP server, CLI and JSON-lines protocol; compact page outlines; approvals for payments; CAPTCHA and sign-in hand-off; audit log.
- Privacy: Brave adblock engine with 18 lists, cosmetic filtering and YouTube ad removal; fingerprint noise; site permissions that ask (Keep blocked, Allow this time, Always allow).
- New: command palette (Ctrl+K), first-run welcome and import (Chrome, Edge, Brave, Firefox; bookmarks, history, passwords incl. Firefox NSS, exact open tabs), dark mode for websites, selection and page translation with a "Translate into" setting, report a problem, prompt-injection scanner, animated New Tab demo.
- Removed: the VPN (the Settings card says "coming soon").
- Faster debug builds (dependencies are optimised) and a retrying translation request.
- Known limits: not Authenticode-signed; Chrome v20 (app-bound) passwords can't be imported directly (use CSV).