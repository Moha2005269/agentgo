// AgentGo client for the local HTTP and WebSocket transport (Node 22+).
import { readFileSync } from "node:fs";

export class AgentGoError extends Error {
  constructor(error) {
    super(`${error.code}: ${error.message}`);
    this.code = error.code;
    this.data = error.data;
  }
}

export class AgentGo {
  constructor({ port, token, client = "typescript" }) {
    this.base = `http://127.0.0.1:${port}`;
    this.token = token;
    this.client = client;
  }

  static async connect({ port, token, tokenFile, client }) {
    const secret = token ?? readFileSync(tokenFile, "utf8").trim();
    return new AgentGo({ port, token: secret, client });
  }

  async call(method, params = {}) {
    const response = await fetch(`${this.base}/v1/call`, {
      method: "POST",
      headers: { authorization: `Bearer ${this.token}`, "content-type": "application/json", "x-agentgo-client": this.client },
      body: JSON.stringify({ method, params }),
    });
    const reply = await response.json();
    if (!response.ok) throw new AgentGoError({ code: "unauthorized", message: reply.error || response.statusText });
    if (!reply.ok) throw new AgentGoError(reply.error);
    return reply.result;
  }

  // A WebSocket session: call() over the socket, onEvent for pushed events.
  async socket() {
    const ws = new WebSocket(`${this.base.replace("http", "ws")}/v1/ws?token=${this.token}`);
    await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
    const pending = new Map();
    let next = 1;
    const session = {
      onEvent: () => {},
      call: (method, params = {}) => new Promise((resolve, reject) => {
        const id = next++;
        pending.set(id, { resolve, reject });
        ws.send(JSON.stringify({ v: 1, id, method, params }));
      }),
      close: () => ws.close(),
    };
    ws.onmessage = (message) => {
      const reply = JSON.parse(message.data);
      const waiting = pending.get(reply.id);
      if (!waiting) { session.onEvent(reply); return; }
      pending.delete(reply.id);
      if (reply.ok) waiting.resolve(reply.result); else waiting.reject(new AgentGoError(reply.error));
    };
    return session;
  }
}
