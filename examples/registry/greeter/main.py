#!/usr/bin/env python3
"""greeter — a tiny example module used to demo the GitHub/package manager.

Provides `greet.hello` with one method, `hi`. No dependencies.
"""
import json
import sys


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def handle(method, params):
    if method in ("initialize", "describe"):
        return {"name": "greeter", "capabilities": ["greet.hello"]}
    if method == "invoke":
        if params.get("method") == "hi":
            name = (params.get("params") or {}).get("name", "stranger")
            return {"message": f"hi {name}, greeter v1 here"}
        raise RuntimeError(f"unknown method {params.get('method')}")
    if method == "shutdown":
        return None
    raise RuntimeError(f"unknown method {method}")


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        if "method" not in msg:
            continue
        rid = msg.get("id")
        try:
            result, error = handle(msg["method"], msg.get("params") or {}), None
        except Exception as exc:  # noqa: BLE001
            result, error = None, {"code": -32000, "message": str(exc)}
        if rid is not None:
            resp = {"jsonrpc": "2.0", "id": rid}
            resp["error" if error else "result"] = error or result
            send(resp)
        if msg["method"] == "shutdown":
            break


if __name__ == "__main__":
    main()
