"""Loopback-only stdio MCP fixture; side effects stay in test temp dirs."""
import json
import os
from pathlib import Path
import sys
import time

label, marker, mode, starts = sys.argv[1:5]
if starts != "-":
    with open(starts, "a", encoding="utf-8") as output:
        output.write("started\n")


def send(value):
    print(json.dumps(value), flush=True)


for line in sys.stdin:
    message = json.loads(line)
    method, identity = message.get("method"), message.get("id")
    if method == "initialize":
        if mode == "held_initialize":
            time.sleep(60)
        send({"jsonrpc": "2.0", "id": identity, "result": {
            "protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
            "serverInfo": {"name": "snapshot", "version": "1"},
            "instructions": label}})
    elif method == "tools/list":
        if mode == "held_list":
            time.sleep(60)
        send({"jsonrpc": "2.0", "id": identity, "result": {"tools": [{
            "name": "record", "description": label,
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": False, "idempotentHint": False}}]}})
    elif method == "tools/call":
        if mode == "held_call":
            Path(marker + ".ready").write_text("ready", encoding="utf-8")
            deadline = time.monotonic() + 10
            while not Path(marker).exists() and time.monotonic() < deadline:
                time.sleep(0.01)
        count = 0
        if marker != "-":
            with open(marker, "a", encoding="utf-8") as output:
                output.write("executed\n")
            count = len(Path(marker).read_text(encoding="utf-8").splitlines())
        if count == 1 and mode == "held_after_commit":
            Path(marker + ".ready").write_text(str(os.getpid()), encoding="utf-8")
            time.sleep(60)
        if count == 1 and mode == "disconnect_once":
            sys.exit(0)
        if count == 1 and mode == "error_once":
            send({"jsonrpc": "2.0", "id": identity, "error": {
                "code": -32603, "message": "Action committed; response preparation failed"}})
        else:
            text = json.dumps({"label": label, "cwd": os.getcwd(), "count": count})
            send({"jsonrpc": "2.0", "id": identity, "result": {
                "content": [{"type": "text", "text": text}], "isError": False}})
    elif identity is not None:
        send({"jsonrpc": "2.0", "id": identity, "error": {
            "code": -32601, "message": "Unknown method"}})
