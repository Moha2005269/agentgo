# AgentGo for TypeScript and JavaScript

```js
import { AgentGo } from "./agentgo.mjs";
const ag = await AgentGo.connect({ port, tokenFile });
const page = await ag.call("navigate.url", { url: "https://example.com" });
const ws = await ag.socket();            // replies and events over WebSocket
```

Errors throw `AgentGoError` with `code` (such as `approval.required`) and `data`.
