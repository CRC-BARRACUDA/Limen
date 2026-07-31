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


class File(Widget):
    """A filesystem path input keyed by `id` (the chosen path comes back in the
    method params, exactly like `Text`).

    The user can type a path, drag a file or folder onto the field, or press
    Browse for the OS picker. `directory=True` picks a folder instead of a file
    and only accepts folders when dropped. `browse` is the button's label — it
    lives here, rather than in the host, so a module can translate it alongside
    the rest of its view."""

    def __init__(self, id, label="", placeholder="", default="", directory=False,
                 browse=""):
        self.id, self.label, self.placeholder = id, label, placeholder
        self.default, self.directory, self.browse = default, bool(directory), browse

    def to_spec(self):
        return {
            "kind": "file", "id": self.id, "label": self.label,
            "placeholder": self.placeholder, "default": self.default,
            "directory": self.directory, "browse": self.browse,
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


class Checkbox(Widget):
    """An on/off checkbox keyed by `id`; its boolean state comes back in the
    method params. Unchecked unless `default=True`."""

    def __init__(self, id, label="", default=False):
        self.id, self.label, self.default = id, label, default

    def to_spec(self):
        return {"kind": "checkbox", "id": self.id, "label": self.label,
                "default": bool(self.default)}


class Button(Widget):
    """A button. `calls` names a method on THIS module; the SDK fills in the
    capability. Use `capability=`/`method=` to target another module instead.
    `primary=True` is the filled accent button, otherwise an outline button; both
    animate on hover/press on the host."""

    def __init__(self, text, calls=None, capability=None, method=None, primary=False,
                 enabled=True, args=None, open_in_tab=False):
        self.text = text
        self.calls = calls
        self.capability = capability
        self.method = method
        self.primary = primary
        self.enabled = enabled
        self.args = args
        self.open_in_tab = open_in_tab

    def _spec(self, own_capability):
        cap = self.capability or own_capability
        meth = self.method or self.calls
        spec = {
            "kind": "button", "text": self.text,
            "style": "primary" if self.primary else "default",
            "enabled": self.enabled,
            "action": {"capability": cap, "method": meth},
        }
        if self.args:
            spec["args"] = self.args
        if self.open_in_tab:
            spec["open_in_tab"] = True
        return spec

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


class MenuItem:
    """A row right-click menu entry.

    Leaf: pass `capability` + `method` — chosen, it invokes that method with the
    activated row's id as the `id` param (plus any `args`). Submenu: pass
    `children` (a list of MenuItems); its own action is then ignored.
    """

    def __init__(self, label, capability=None, method=None, args=None,
                 open_in_tab=False, children=None):
        self.label = label
        self.capability = capability
        self.method = method
        self.args = args
        self.open_in_tab = open_in_tab
        self.children = children

    def to_spec(self):
        spec = {"label": str(self.label)}
        if self.children:
            spec["children"] = [c.to_spec() for c in self.children]
        elif self.capability is not None and self.method is not None:
            spec["action"] = {"capability": self.capability, "method": self.method}
        if self.args:
            spec["args"] = self.args
        if self.open_in_tab:
            spec["open_in_tab"] = True
        return spec


class Table(Widget):
    """A table with a header row (`columns`) and string cells (`rows`).

    Rows become interactive when `row_ids` is given together with `menu` and/or
    `on_activate`: right-click shows the `menu` (a list of `MenuItem`), and
    double-click runs `on_activate` — a `(capability, method)` tuple whose
    returned view opens in a new tab. The activated row's id is sent as `id`.
    """

    def __init__(self, columns, rows, row_ids=None, menu=None, on_activate=None,
                 row_menus=None):
        self.columns, self.rows = columns, rows
        self.row_ids = row_ids
        self.menu = menu
        self.on_activate = on_activate
        # Optional per-row menus (parallel to rows); a non-empty entry overrides
        # `menu` for that row.
        self.row_menus = row_menus

    def to_spec(self):
        spec = {
            "kind": "table",
            "columns": [str(c) for c in self.columns],
            "rows": [[str(c) for c in row] for row in self.rows],
        }
        if self.row_ids is not None:
            spec["row_ids"] = [str(i) for i in self.row_ids]
        if self.menu:
            spec["menu"] = [mi.to_spec() for mi in self.menu]
        if self.row_menus:
            spec["row_menus"] = [[mi.to_spec() for mi in m] for m in self.row_menus]
        if self.on_activate is not None:
            cap, method = self.on_activate
            spec["on_activate"] = {
                "action": {"capability": cap, "method": method},
                "open_in_tab": True,
            }
        return spec


class Chart(Widget):
    """A horizontal bar chart: `data` is a list of (label, value) pairs, drawn
    as bars scaled to the largest value, under an optional `title`."""

    def __init__(self, title, data):
        self.title, self.data = title, data

    def to_spec(self):
        return {
            "kind": "chart",
            "title": str(self.title),
            "data": [{"label": str(label), "value": float(value)} for (label, value) in self.data],
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

    def capabilities(self):
        """Every capability currently provided by a loaded module. Use this to
        discover OPTIONAL integrations — e.g. only show a feature when another
        module (a `report.*` provider, say) is present."""
        return self._m._request("host.capabilities", None) or []

    def has_capability(self, capability):
        """Whether some loaded module provides `capability` (exact match)."""
        return capability in self.capabilities()

    def open(self, target, value=""):
        """Ask the host to open something in the OS on the user's behalf:
        `target` is "path" | "url" | "registry" | "device_manager"; `value` is
        the path / URL / registry key (ignored for device_manager). Best-effort;
        registry and device_manager are Windows-only."""
        self._m._request("host.open", {"target": target, "value": value})

    def pick_file(self):
        """Show a native 'open file' dialog on the host; returns the chosen path,
        or None if the user cancelled."""
        r = self._m._request("host.pick_file", None)
        return r.get("path") if isinstance(r, dict) else None

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

    def _own_capability(self):
        """The capability a Button defaults to. Set before *any* serialization —
        a method that returns a new view is as common as the `ui` handler, and
        buttons in it would otherwise carry an empty capability and do nothing."""
        _CURRENT_CAPABILITY[0] = self.capabilities[0] if self.capabilities else ""

    def _view_spec(self):
        if self._ui is None:
            raise RuntimeError("this module has no UI")
        self._own_capability()
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
            result = self._handle(method, msg.get("params") or {})
            # A method may return a Window/Widget object directly (like the ui
            # handler); serialize it to its spec so it's JSON-encodable.
            if hasattr(result, "to_spec"):
                self._own_capability()
                result = result.to_spec()
            error = None
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
