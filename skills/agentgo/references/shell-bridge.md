# Shell bridge reference

The trusted shell (`agentgo://shell`) calls native code with:

```js
const result = await window.agentgo.request({ id, method, params });
window.agentgo.subscribe(event => render(event));
```

- `method` must exist in the `agentgo-shell-bridge` registry
  (`crates/agentgo-shell-bridge/src/methods.rs`), which is generated in
  lockstep with `schemas/capability-manifest.json`.
- Responses use the same envelope/error codes as every other transport
  (`docs/protocol.md`).
- Native state is authoritative; UI state is a projection. The UI may
  optimistically animate safe cosmetic state but must reconcile with
  native results.
- Unsupported methods return `capability.unsupported` — the UI must
  render those controls disabled with an honest reason, never a
  success toast.
- The bridge is reachable only from the exact `agentgo://shell` origin
  main frame (see `crates/agentgo-shell-bridge/src/origin.rs`).
