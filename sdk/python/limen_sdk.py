"""Limen Python SDK.

Write a module by declaring handlers — the SDK runs the JSON-RPC stdio loop,
answers the host's lifecycle methods, serves your UI, brokers calls to other
modules, and delivers events.

    from limen_sdk import Module, Window, Label, Text, Button

    m = Module("greeter", capabilities=["demo.greet"])

    @m.method("hi")
    def hi(params, host):
        return {"message": f"hi {params.get('name', 'there')}"}

    @m.ui
    def ui():
        return Window("Greeter", [
            Text("name", label="Name"),
            Button("Say hi", calls="hi", primary=True),
        ])

    m.run()

The SDK is injected by the host (on PYTHONPATH); modules never vendor it.
"""
import json
import sys

__all__ = [
    "Module", "Host",
    "Window", "Label", "Text", "Select", "Button", "Row", "Separator", "Table",
]


# --------------------------------------------------------------------------- #
# UI builder — constructs the declarative view spec the GUI core renders.
# --------------------------------------------------------------------------- #

class Widget:
    """Base class for view widgets. Subclasses build the JSON `to_spec()` the host
    renders — with its shared animated styling, so a module's UI animates like the
    host's chrome."""

    def to_spec(self):
        raise NotImplementedError


class Label(Widget):
    """A text label. `style` is one of "normal", "heading", "strong", "weak",
    "mono"."""

    def __init__(self, text, style="normal"):
        self.text, self.style = text, style

    def to_spec(self):
        return {"kind": "label", "text": self.text, "style": self.style}


class Text(Widget):
    """A text input keyed by `id` (its value comes back in the method params).
    `multiline` makes it a box, `password` masks it. Single-line fields get an
    animated focus border on the host."""

    def __init__(self, id, label="", placeholder="", multiline=False, default="", password=False):
        self.id, self.label = id, label
        self.placeholder, self.multiline, self.default = placeholder, multiline, default
        self.password = password

    def to_spec(self):
        return {
            "kind": "text", "id": self.id, "label": self.label,
            "placeholder": self.placeholder, "multiline": self.multiline,
            "default": self.default, "password": self.password,
        }


class Select(Widget):
    """A dropdown keyed by `id`; `options` are the choices and the selection comes
    back in the method params."""

    def __init__(self, id, options, label="", default=""):
        self.id, self.options, self.label, self.default = id, options, label, default

    def to_spec(self):
        return {
            "kind": "select", "id": self.id, "label": self.label,
            "options": list(self.options), "default": self.default,
        }


class Button(Widget):
    """A button. `calls` names a method on THIS module; the SDK fills in the
    capability. Use `capability=`/`method=` to target another module instead.
    `primary=True` is the filled accent button, otherwise an outline button; both
    animate on hover/press on the host."""

    def __init__(self, text, calls=None, capability=None, method=None, primary=False,
                 enabled=True):
        self.text = text
        self.calls = calls
        self.capability = capability
        self.method = method
        self.primary = primary
        self.enabled = enabled

    def _spec(self, own_capability):
        cap = self.capability or own_capability
        meth = self.method or self.calls
        return {
            "kind": "button", "text": self.text,
            "style": "primary" if self.primary else "default",
            "enabled": self.enabled,
            "action": {"capability": cap, "method": meth},
        }

    # to_spec needs the owning capability; provided during serialization.
    def to_spec(self):
        return self._spec(_CURRENT_CAPABILITY[0])


class Separator(Widget):
    """A horizontal divider."""

    def to_spec(self):
        return {"kind": "separator"}


class Row(Widget):
    """A horizontal group of widgets, laid out left to right."""

    def __init__(self, children):
        self.children = children

    def to_spec(self):
        return {"kind": "row", "children": [c.to_spec() for c in self.children]}


class Table(Widget):
    """A table with a header row (`columns`) and string cells (`rows`)."""

    def __init__(self, columns, rows):
        self.columns, self.rows = columns, rows

    def to_spec(self):
        return {
            "kind": "table",
            "columns": [str(c) for c in self.columns],
            "rows": [[str(c) for c in row] for row in self.rows],
        }


class Window:
    """Top-level view: a title and a list of widgets."""

    def __init__(self, title, widgets):
        self.title, self.widgets = title, widgets

    def to_spec(self):
        return {"title": self.title, "widgets": [w.to_spec() for w in self.widgets]}


# The capability a Button defaults to (set while serializing a module's view).
_CURRENT_CAPABILITY = [""]


# --------------------------------------------------------------------------- #
# Host bridge — call other modules, emit/subscribe events, log.
# --------------------------------------------------------------------------- #

class Host:
    def __init__(self, module):
        self._m = module

    def call(self, capability, method, params=None):
        """Invoke a method on whichever module provides `capability`, and block
        for the result (routed by the broker)."""
        return self._m._request(
            "host.call",
            {"capability": capability, "method": method, "params": params or {}},
        )

    def emit(self, topic, payload=None):
        """Fire an event to every subscriber of `topic` (fire-and-forget)."""
        return self._m._request("host.emit", {"topic": topic, "payload": payload})

    def subscribe(self, topic):
        """Subscribe this module to `topic`. Usually done via @m.on(topic)."""
        return self._m._request(
            "host.subscribe", {"subscriber": self._m.name, "topic": topic}
        )

    def about(self):
        """Info about the host environment: os, arch, family, hostname,
        limen_version, base_dir (where Limen runs from)."""
        return self._m._request("host.about", {})

    def log(self, message):
        self._m._request("host.log", str(message))


# --------------------------------------------------------------------------- #
# Module — the loop.
# --------------------------------------------------------------------------- #

class Module:
    def __init__(self, name, capabilities):
        self.name = name
        self.capabilities = list(capabilities)
        self.host = Host(self)
        self._methods = {}      # method name -> fn(params, host)
        self._events = {}       # topic -> fn(payload, host)
        self._ui = None         # fn() -> Window/spec
        self._next_id = 0

    # ---- registration ----------------------------------------------------- #

    def method(self, name):
        def deco(fn):
            self._methods[name] = fn
            return fn
        return deco

    def on(self, topic):
        """Register an event handler; subscribes on startup."""
        def deco(fn):
            self._events[topic] = fn
            return fn
        return deco

    def ui(self, fn):
        self._ui = fn
        return fn

    # ---- io --------------------------------------------------------------- #

    def _send(self, obj):
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()

    def _request(self, method, params):
        """Send a request to the host and block for its response — while waiting,
        still dispatch any inbound frames (events / re-entrant calls) so nothing
        deadlocks."""
        self._next_id += 1
        rid = self._next_id
        self._send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        while True:
            msg = self._read()
            if msg is None:
                raise RuntimeError("host closed the connection")
            if "method" not in msg and msg.get("id") == rid:
                if msg.get("error"):
                    raise RuntimeError(msg["error"]["message"])
                return msg.get("result")
            # Anything else arriving mid-call (an event, or a request from the
            # host) is dispatched now so we stay responsive and deadlock-free.
            self._dispatch(msg)

    def _read(self):
        line = sys.stdin.readline()
        while line and not line.strip():
            line = sys.stdin.readline()
        if not line:
            return None
        return json.loads(line)

    # ---- dispatch --------------------------------------------------------- #

    def _view_spec(self):
        if self._ui is None:
            raise RuntimeError("this module has no UI")
        _CURRENT_CAPABILITY[0] = self.capabilities[0] if self.capabilities else ""
        view = self._ui()
        return view.to_spec() if hasattr(view, "to_spec") else view

    def _handle(self, method, params):
        if method in ("initialize", "describe"):
            return {"name": self.name, "capabilities": self.capabilities}
        if method == "invoke":
            inner = params.get("method")
            inner_params = params.get("params") or {}
            if inner == "ui":
                return self._view_spec()
            fn = self._methods.get(inner)
            if fn is None:
                raise RuntimeError(f"unknown method {inner}")
            return fn(inner_params, self.host)
        if method == "shutdown":
            return None
        raise RuntimeError(f"unknown method {method}")

    def _dispatch(self, msg):
        """Handle one inbound frame. Returns True if it was a `shutdown`."""
        method = msg.get("method")
        if method is None:
            return False  # a stray response; ignore
        # Events are notifications the host pushes: {method:"event", params:{topic,payload}}
        if method == "event" and msg.get("id") is None:
            p = msg.get("params") or {}
            handler = self._events.get(p.get("topic"))
            if handler is not None:
                try:
                    handler(p.get("payload"), self.host)
                except Exception as exc:  # noqa: BLE001
                    self.host.log(f"{self.name}: event handler error: {exc}")
            return False

        rid = msg.get("id")
        try:
            result, error = self._handle(method, msg.get("params") or {}), None
        except Exception as exc:  # noqa: BLE001
            result, error = None, {"code": -32000, "message": str(exc)}
        if rid is not None:
            resp = {"jsonrpc": "2.0", "id": rid}
            resp["error" if error else "result"] = error or result
            self._send(resp)
        return method == "shutdown"

    # ---- main loop -------------------------------------------------------- #

    def run(self):
        # Subscribe to every registered event topic.
        for topic in self._events:
            try:
                self.host.subscribe(topic)
            except Exception as exc:  # noqa: BLE001
                self.host.log(f"{self.name}: subscribe {topic} failed: {exc}")

        while True:
            msg = self._read()
            if msg is None:
                break
            if self._dispatch(msg):
                break
