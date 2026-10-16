# AgentGo for Python

```python
from agentgo import AgentGo, AgentGoError
ag = AgentGo(port, token_file=path)
page = ag.call("navigate.url", {"url": "https://example.com"})
```

Standard library only. Errors raise `AgentGoError` with `code` and `data`.
`python agentgo.py <port> <token file> <method> [params JSON]` tries it.
