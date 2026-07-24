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
    def to_spec(self) -> Dict[str, Any]: ...

class Label(Widget):
    def __init__(self, text: str, style: str = ...) -> None: ...

class Text(Widget):
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
    def __init__(
        self, id: str, options: List[str], label: str = ..., default: str = ...
    ) -> None: ...

class Button(Widget):
    def __init__(
        self,
        text: str,
        calls: Optional[str] = ...,
        capability: Optional[str] = ...,
        method: Optional[str] = ...,
        primary: bool = ...,
        enabled: bool = ...,
    ) -> None: ...

class Separator(Widget): ...

class Table(Widget):
    def __init__(self, columns: list[str], rows: list[list[str]]) -> None: ...

class Row(Widget):
    def __init__(self, children: List[Widget]) -> None: ...

class Window:
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
