"""AgentGo client for the local HTTP transport (standard library only)."""
import json
import urllib.error
import urllib.request


class AgentGoError(Exception):
    def __init__(self, error):
        super().__init__(f"{error.get('code')}: {error.get('message')}")
        self.code = error.get("code")
        self.data = error.get("data")


class AgentGo:
    def __init__(self, port, token=None, token_file=None, client="python"):
        if token is None:
            with open(token_file, encoding="utf8") as handle:
                token = handle.read().strip()
        self.url = f"http://127.0.0.1:{port}/v1/call"
        self.token = token
        self.client = client

    def call(self, method, params=None, timeout=600):
        body = json.dumps({"method": method, "params": params or {}}).encode()
        request = urllib.request.Request(self.url, data=body, method="POST", headers={
            "Authorization": f"Bearer {self.token}",
            "Content-Type": "application/json",
            "X-AgentGo-Client": self.client,
        })
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                reply = json.loads(response.read())
        except urllib.error.HTTPError as error:
            raise AgentGoError({"code": "unauthorized", "message": error.read().decode()}) from None
        if not reply.get("ok"):
            raise AgentGoError(reply.get("error", {}))
        return reply.get("result")


if __name__ == "__main__":
    import sys
    port, token_file, method = int(sys.argv[1]), sys.argv[2], sys.argv[3]
    params = json.loads(sys.argv[4]) if len(sys.argv) > 4 else {}
    print(json.dumps(AgentGo(port, token_file=token_file).call(method, params)))
