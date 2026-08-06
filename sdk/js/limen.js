// Limen JavaScript SDK (Node).
//
// Mirrors the Python SDK: declare handlers, let the SDK run the JSON-RPC stdio
// loop, serve your UI, broker calls to other modules, and deliver events.
//
//   const { Module, Window, Text, Button } = require("limen");
//
//   const m = new Module("hello", ["demo.hello"]);
//   m.method("greet", (params, host) => ({ msg: `hi ${params.name || "there"}` }));
//   m.ui(() => Window("Hello", [ Text("name", { label: "Name" }),
//                                Button("Greet", { calls: "greet", primary: true }) ]));
//   m.run();
//
// The host injects this SDK (on NODE_PATH); modules never vendor it.

"use strict";
const fs = require("fs");

// A tiny synchronous sleep (used only if stdin reports EAGAIN).
const _sab = new Int32Array(new SharedArrayBuffer(4));
function _msleep(ms) {
  try { Atomics.wait(_sab, 0, 0, ms); } catch (_) { /* ignore */ }
}

// --------------------------------------------------------------------------- //
// UI builder — constructs the declarative view spec the GUI core renders.
// --------------------------------------------------------------------------- //

let _currentCapability = "";

function Label(text, opts = {}) {
  return { toSpec: () => ({ kind: "label", text, style: opts.style || "normal" }) };
}
function Text(id, opts = {}) {
  return { toSpec: () => ({
    kind: "text", id, label: opts.label || "", placeholder: opts.placeholder || "",
    multiline: !!opts.multiline, default: opts.default || "",
  }) };
}
// A filesystem path input keyed by `id` (the chosen path comes back in the
// method params, like Text). Typed, dropped onto, or chosen via Browse.
// `accepts` is "file", "dir", or "file|dir" for either; a dual field shows two
// Browse buttons, since no OS has a dialog that picks either — label the second
// with `browseDir`. Both labels live here so the module can translate them.
function File(id, opts = {}) {
  return { toSpec: () => ({
    kind: "file", id, label: opts.label || "", placeholder: opts.placeholder || "",
    default: opts.default || "",
    // `directory: true` is the older spelling of `accepts: "dir"`.
    accepts: opts.directory ? "dir" : (opts.accepts || "file"),
    browse: opts.browse || "",
    browse_dir: opts.browseDir || opts.browse_dir || "",
  }) };
}
function Select(id, options, opts = {}) {
  return { toSpec: () => ({
    kind: "select", id, options: options.slice(), label: opts.label || "", default: opts.default || "",
  }) };
}
function Button(text, opts = {}) {
  return { toSpec: () => ({
    kind: "button", text,
    // `danger` is the red one, for an action that destroys something.
    style: opts.danger ? "danger" : (opts.primary ? "primary" : "default"),
    action: { capability: opts.capability || _currentCapability, method: opts.method || opts.calls },
  }) };
}
function Separator() {
  return { toSpec: () => ({ kind: "separator" }) };
}
function Row(children) {
  return { toSpec: () => ({ kind: "row", children: children.map((c) => c.toSpec()) }) };
}
function Window(title, widgets) {
  return { toSpec: () => ({ title, widgets: widgets.map((w) => w.toSpec()) }) };
}

// --------------------------------------------------------------------------- //
// Host bridge — call other modules, emit/subscribe events, log.
// --------------------------------------------------------------------------- //

class Host {
  constructor(module) { this._m = module; }
  call(capability, method, params = {}) {
    return this._m._request("host.call", { capability, method, params });
  }
  emit(topic, payload = null) {
    return this._m._request("host.emit", { topic, payload });
  }
  subscribe(topic) {
    return this._m._request("host.subscribe", { subscriber: this._m.name, topic });
  }
  about() {
    // { os, arch, family, hostname, limen_version, base_dir }
    return this._m._request("host.about", {});
  }
  // This module's own directory on disk. Content the module fetches for itself
  // belongs under `tools/` in here: excluded from the trust digest, removed with
  // the module, wiped when the module updates.
  moduleDir() {
    const r = this._m._request("host.module_dir", null);
    return typeof r === "string" && r ? r : null;
  }
  // Raise a desktop notification on the machine running Limen, for work the
  // user is not watching. `urgency` is "low" | "normal" | "critical".
  // Best-effort: a session with no notification daemon shows nothing.
  notify(title, body = "", urgency = "normal") {
    this._m._request("host.notify", { title: String(title), body: String(body), urgency });
  }
  log(message) {
    this._m._request("host.log", String(message));
  }
}

// --------------------------------------------------------------------------- //
// Module — the loop.
// --------------------------------------------------------------------------- //

class Module {
  constructor(name, capabilities) {
    this.name = name;
    this.capabilities = capabilities.slice();
    this.host = new Host(this);
    this._methods = {};
    this._events = {};
    this._ui = null;
    this._nextId = 0;
    this._buf = "";
  }

  method(name, fn) { this._methods[name] = fn; return this; }
  on(topic, fn) { this._events[topic] = fn; return this; }
  ui(fn) { this._ui = fn; return this; }

  // ---- io --------------------------------------------------------------- //

  _send(obj) { process.stdout.write(JSON.stringify(obj) + "\n"); }

  // Blocking read of one line from stdin (fd 0).
  _readLine() {
    for (;;) {
      const nl = this._buf.indexOf("\n");
      if (nl >= 0) {
        const line = this._buf.slice(0, nl);
        this._buf = this._buf.slice(nl + 1);
        return line;
      }
      const chunk = Buffer.allocUnsafe(4096);
      let n;
      try {
        n = fs.readSync(0, chunk, 0, chunk.length, null);
      } catch (e) {
        if (e.code === "EAGAIN") { _msleep(5); continue; }
        if (e.code === "EOF") { n = 0; } else { throw e; }
      }
      if (n === 0) {
        if (this._buf.length) { const line = this._buf; this._buf = ""; return line; }
        return null; // EOF
      }
      this._buf += chunk.toString("utf8", 0, n);
    }
  }

  // Send a request to the host and block for its response — dispatching any
  // inbound frames (events / re-entrant calls) that arrive meanwhile.
  _request(method, params) {
    this._nextId += 1;
    const id = this._nextId;
    this._send({ jsonrpc: "2.0", id, method, params });
    for (;;) {
      const line = this._readLine();
      if (line === null) throw new Error("host closed the connection");
      if (!line.trim()) continue;
      const msg = JSON.parse(line);
      if (!("method" in msg) && msg.id === id) {
        if (msg.error) throw new Error(msg.error.message);
        return msg.result;
      }
      this._dispatch(msg);
    }
  }

  // ---- dispatch --------------------------------------------------------- //

  _viewSpec() {
    if (!this._ui) throw new Error("this module has no UI");
    _currentCapability = this.capabilities[0] || "";
    const view = this._ui();
    return view && view.toSpec ? view.toSpec() : view;
  }

  _handle(method, params) {
    if (method === "initialize" || method === "describe") {
      return { name: this.name, capabilities: this.capabilities };
    }
    if (method === "invoke") {
      const inner = params.method;
      const innerParams = params.params || {};
      if (inner === "ui") return this._viewSpec();
      const fn = this._methods[inner];
      if (!fn) throw new Error(`unknown method ${inner}`);
      return fn(innerParams, this.host);
    }
    if (method === "shutdown") return null;
    throw new Error(`unknown method ${method}`);
  }

  // Handle one inbound frame. Returns true if it was a shutdown.
  _dispatch(msg) {
    const method = msg.method;
    if (method === undefined) return false; // stray response
    if (method === "event" && msg.id === undefined) {
      const p = msg.params || {};
      const handler = this._events[p.topic];
      if (handler) {
        try { handler(p.payload, this.host); }
        catch (e) { this.host.log(`${this.name}: event handler error: ${e.message}`); }
      }
      return false;
    }
    const id = msg.id;
    let result = null, error = null;
    try { result = this._handle(method, msg.params || {}); }
    catch (e) { error = { code: -32000, message: e.message }; }
    if (id !== undefined && id !== null) {
      const resp = { jsonrpc: "2.0", id };
      if (error) resp.error = error; else resp.result = result;
      this._send(resp);
    }
    return method === "shutdown";
  }

  // ---- main loop -------------------------------------------------------- //

  run() {
    for (const topic of Object.keys(this._events)) {
      try { this.host.subscribe(topic); }
      catch (e) { this.host.log(`${this.name}: subscribe ${topic} failed: ${e.message}`); }
    }
    for (;;) {
      const line = this._readLine();
      if (line === null) break;
      if (!line.trim()) continue;
      if (this._dispatch(JSON.parse(line))) break;
    }
  }
}

module.exports = { Module, Host, Window, Label, Text, File, Select, Button, Row, Separator };
