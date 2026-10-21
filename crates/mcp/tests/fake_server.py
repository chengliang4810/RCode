import json, sys, threading

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        send({"jsonrpc":"2.0","id":mid,"result":{
            "protocolVersion":"2025-06-18",
            "capabilities":{"tools":{}},
            "serverInfo":{"name":"fake","version":"1.0"},
            "instructions":"Use the echo tool to repeat the supplied text."}})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc":"2.0","id":mid,"result":{"tools":[
            {"name":"echo","description":"Echo back the text.",
             "inputSchema":{"type":"object","properties":{"text":{"type":"string"}},
                            "required":["text"]},
             "annotations":{"readOnlyHint":True}}]}})
    elif method == "tools/call":
        args = msg.get("params",{}).get("arguments",{})
        send({"jsonrpc":"2.0","id":mid,"result":{
            "content":[{"type":"text","text":"echo: " + str(args.get("text",""))}],
            "isError": False}})
    elif mid is not None:
        send({"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"no such method"}})
