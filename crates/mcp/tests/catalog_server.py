"""Local MCP catalog fixture with distinct schemas and recorded call identities."""
import json
from pathlib import Path
import sys

label, marker, tool_names = sys.argv[1:4]
tool_names = json.loads(tool_names)


def send(value):
    print(json.dumps(value), flush=True)


for line in sys.stdin:
    message = json.loads(line)
    method, identity = message.get("method"), message.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": identity, "result": {
            "protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
            "serverInfo": {"name": label, "version": "1"}}})
    elif method == "tools/list":
        tools = [{"name": name, "description": label, "inputSchema": {
            "type": "object", "properties": {"label": {"const": label}},
            "required": ["label"]}} for name in tool_names]
        send({"jsonrpc": "2.0", "id": identity, "result": {"tools": tools}})
    elif method == "tools/call":
        params = message["params"]
        assert params["name"] in tool_names
        assert params["arguments"]["label"] == label
        result = {"label": label, "name": params["name"]}
        with Path(marker).open("a", encoding="utf-8") as output:
            output.write(json.dumps(result) + "\n")
        send({"jsonrpc": "2.0", "id": identity, "result": {
            "content": [{"type": "text", "text": json.dumps(result)}]}})
    elif identity is not None:
        send({"jsonrpc": "2.0", "id": identity, "error": {
            "code": -32601, "message": "Unknown method"}})
