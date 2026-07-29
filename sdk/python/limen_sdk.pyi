"""Type stubs for the Limen Python SDK — for editor autocomplete / type checking.

The runtime module is host-injected; this stub just describes its API. Point your
editor at this directory (e.g. add `<Limen>/sdk/python` to your analysis paths).
"""
from typing import Any, Callable, Dict, List, Optional

__all__ = [
    "Module", "Host",
    "Window", "Label", "Text", "Select", "Button", "Row", "Separator",
]

# ---- UI builder ----------------------------------------------------------- #

class Widget:
    """Base class for view widgets. The host renders them with its shared animated
    styling, so a module's UI animates like the host's chrome."""
    def to_spec(self) -> Dict[str, Any]: ...

class Label(Widget):
    """A text label. `style`: "normal" | "heading" | "strong" | "weak" | "mono"."""
    def __init__(self, text: str, style: str = ...) -> None: ...

class Text(Widget):
    """A text input keyed by `id` (value returned in the method params). `multiline`
    makes it a box, `password` masks it; single-line fields get an animated focus
    border on the host."""
    def __init__(
        self,
        id: str,
        label: str = ...,
        placeholder: str = ...,
        multiline: bool = ...,
        default: str = ...,
        password: bool = ...,
    ) -> None: ...

class Select(Widget):
    """A dropdown keyed by `id`; `options` are the choices, selection returned in
    the method params."""
    def __init__(
        self, id: str, options: List[str], label: str = ..., default: str = ...
    ) -> None: ...

class Checkbox(Widget):
    """An on/off checkbox keyed by `id`; boolean state returned in the params."""
    def __init__(self, id: str, label: str = ..., default: bool = ...) -> None: ...

class Button(Widget):
    """A button. `calls` names a method on THIS module (SDK fills the capability);
    use `capability`/`method` to target another. `primary=True` is the filled
    accent button, else an outline button; both animate on hover/press."""
    def __init__(
        self,
        text: str,
        calls: Optional[str] = ...,
        capability: Optional[str] = ...,
        method: Optional[str] = ...,
        primary: bool = ...,
        enabled: bool = ...,
        args: Optional[Dict[str, Any]] = ...,
        open_in_tab: bool = ...,
    ) -> None: ...

class Separator(Widget):
    """A horizontal divider."""
    ...

class MenuItem:
    """A row right-click menu entry (leaf action, or a submenu via `children`)."""
    def __init__(
        self,
        label: Any,
        capability: Optional[str] = ...,
        method: Optional[str] = ...,
        args: Optional[Dict[str, Any]] = ...,
        open_in_tab: bool = ...,
        children: Optional[List["MenuItem"]] = ...,
    ) -> None: ...
    def to_spec(self) -> Dict[str, Any]: ...

class Table(Widget):
    """A table with a header row (`columns`) and string cells (`rows`).

    With `row_ids` plus `menu`/`on_activate`, rows become interactive
    (right-click menu + double-click); the activated row's id is sent as `id`.
    """
    def __init__(
        self,
        columns: list[str],
        rows: list[list[str]],
        row_ids: Optional[List[str]] = ...,
        menu: Optional[List[MenuItem]] = ...,
        on_activate: Optional[tuple[str, str]] = ...,
        row_menus: Optional[List[List[MenuItem]]] = ...,
    ) -> None: ...

class Row(Widget):
    """A horizontal group of widgets, laid out left to right."""
    def __init__(self, children: List[Widget]) -> None: ...

class Chart(Widget):
    """A horizontal bar chart: `data` is a list of (label, value) pairs."""
    def __init__(self, title: Any, data: List[tuple[Any, float]]) -> None: ...

class Window:
    """Top-level view: a title and a list of widgets — return from a module's `ui`
    method (or via `@m.ui`)."""
    def __init__(self, title: str, widgets: List[Widget]) -> None: ...
    def to_spec(self) -> Dict[str, Any]: ...

# ---- host bridge ---------------------------------------------------------- #

class Host:
    def call(
        self, capability: str, method: str, params: Optional[Dict[str, Any]] = ...
    ) -> Any: ...
    def emit(self, topic: str, payload: Any = ...) -> Any: ...
    def subscribe(self, topic: str) -> Any: ...
    def about(self) -> Dict[str, Any]: ...
    def capabilities(self) -> List[str]: ...
    def has_capability(self, capability: str) -> bool: ...
    def open(self, target: str, value: str = ...) -> None: ...
    def pick_file(self) -> Optional[str]: ...
    def log(self, message: Any) -> None: ...

# A method handler: (params, host) -> result
Handler = Callable[[Dict[str, Any], Host], Any]
# An event handler: (payload, host) -> None
EventHandler = Callable[[Any, Host], None]
# A UI provider: () -> Window | dict
UiProvider = Callable[[], Any]

# ---- module --------------------------------------------------------------- #

class Module:
    name: str
    capabilities: List[str]
    host: Host

    def __init__(self, name: str, capabilities: List[str]) -> None: ...
    def method(self, name: str) -> Callable[[Handler], Handler]: ...
    def on(self, topic: str) -> Callable[[EventHandler], EventHandler]: ...
    def ui(self, fn: UiProvider) -> UiProvider: ...
    def run(self) -> None: ...
