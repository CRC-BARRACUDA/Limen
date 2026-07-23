#!/usr/bin/env python3
"""caller — example module that depends on `greeter`.

Provides `greet.caller`; its `run` method reaches `greet.hello` (provided by the
greeter module) through the broker.
"""
import json
import sys

_next_id = 0


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def call_host(method, params):
    global _next_id
    _next_id += 1
    rid = _next_id
    send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        if "method" not in msg and msg.get("id") == rid:
            if msg.get("error"):
                raise RuntimeError(msg["error"]["message"])
            return msg.get("result")
    raise RuntimeError("host closed the connection")


def handle(method, params):
    if method in ("initialize", "describe"):
        return {"name": "caller", "capabilities": ["greet.caller"]}
    if method == "invoke":
        if params.get("method") == "run":
            name = (params.get("params") or {}).get("name", "stranger")
            greeting = call_host(
                "host.call",
                {"capability": "greet.hello", "method": "hi", "params": {"name": name}},
            )
            return {"caller": "reached greet.hello via the broker", "greeter_said": greeting}
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
