-- Limen Lua SDK.
--
-- Mirrors the Python/JS SDKs: declare handlers, let the SDK run the JSON-RPC
-- stdio loop, serve your UI, broker calls to other modules, and deliver events.
--
--   local L = require("limen")
--   local m = L.Module("hello", { "demo.hello" })
--
--   m:method("greet", function(params, host)
--     return { msg = "hi " .. (params.name or "there") }
--   end)
--
--   m:ui(function()
--     return L.Window("Hello", {
--       L.Text("name", { label = "Name" }),
--       L.Button("Greet", { calls = "greet", primary = true }),
--     })
--   end)
--
--   m:run()
--
-- The host injects this SDK (on LUA_PATH); modules never vendor it.
-- Lua has no built-in JSON, so a compact encoder/decoder is bundled below.

local M = {}

-- --------------------------------------------------------------------------- --
-- Minimal JSON (encode/decode). Handles the subset Limen uses: objects, arrays,
-- strings, numbers, booleans, null. `null` decodes to a sentinel; nil in tables
-- is omitted on encode.
-- --------------------------------------------------------------------------- --

local json = {}
json.null = setmetatable({}, { __tostring = function() return "null" end })

local function is_array(t)
  local n = 0
  for k in pairs(t) do
    if type(k) ~= "number" then return false end
    n = n + 1
  end
  return n == #t
end

local esc = {
  ['"'] = '\\"', ['\\'] = '\\\\', ['\b'] = '\\b', ['\f'] = '\\f',
  ['\n'] = '\\n', ['\r'] = '\\r', ['\t'] = '\\t',
}

local function encode_str(s)
  return '"' .. s:gsub('[%z\1-\31\\"]', function(c)
    return esc[c] or string.format('\\u%04x', string.byte(c))
  end) .. '"'
end

function json.encode(v)
  local t = type(v)
  if v == json.null or v == nil then
    return "null"
  elseif t == "boolean" then
    return tostring(v)
  elseif t == "number" then
    return tostring(v)
  elseif t == "string" then
    return encode_str(v)
  elseif t == "table" then
    local parts = {}
    if is_array(v) then
      for _, item in ipairs(v) do parts[#parts + 1] = json.encode(item) end
      return "[" .. table.concat(parts, ",") .. "]"
    else
      for k, val in pairs(v) do
        if val ~= nil then
          parts[#parts + 1] = encode_str(tostring(k)) .. ":" .. json.encode(val)
        end
      end
      return "{" .. table.concat(parts, ",") .. "}"
    end
  end
  return "null"
end

-- Recursive-descent decoder.
local decode_value
local function skip_ws(s, i)
  local _, j = s:find("^[ \t\r\n]*", i)
  return (j or i - 1) + 1
end

local function decode_str(s, i)
  local out, j = {}, i + 1
  while j <= #s do
    local c = s:sub(j, j)
    if c == '"' then return table.concat(out), j + 1 end
    if c == "\\" then
      local n = s:sub(j + 1, j + 1)
      local map = { ['"'] = '"', ['\\'] = '\\', ["/"] = "/", b = "\b", f = "\f", n = "\n", r = "\r", t = "\t" }
      if n == "u" then
        local hex = s:sub(j + 2, j + 5)
        out[#out + 1] = utf8 and utf8.char(tonumber(hex, 16)) or "?"
        j = j + 6
      else
        out[#out + 1] = map[n] or n
        j = j + 2
      end
    else
      out[#out + 1] = c
      j = j + 1
    end
  end
  error("unterminated string")
end

decode_value = function(s, i)
  i = skip_ws(s, i)
  local c = s:sub(i, i)
  if c == "{" then
    local obj = {}
    i = skip_ws(s, i + 1)
    if s:sub(i, i) == "}" then return obj, i + 1 end
    while true do
      local key; key, i = decode_str(s, skip_ws(s, i))
      i = skip_ws(s, i)
      i = i + 1 -- ':'
      local val; val, i = decode_value(s, i)
      obj[key] = val
      i = skip_ws(s, i)
      local ch = s:sub(i, i)
      if ch == "," then i = skip_ws(s, i + 1)
      elseif ch == "}" then return obj, i + 1
      else error("bad object") end
    end
  elseif c == "[" then
    local arr = {}
    i = skip_ws(s, i + 1)
    if s:sub(i, i) == "]" then return arr, i + 1 end
    while true do
      local val; val, i = decode_value(s, i)
      arr[#arr + 1] = val
      i = skip_ws(s, i)
      local ch = s:sub(i, i)
      if ch == "," then i = i + 1
      elseif ch == "]" then return arr, i + 1
      else error("bad array") end
    end
  elseif c == '"' then
    return decode_str(s, i)
  elseif c == "t" then return true, i + 4
  elseif c == "f" then return false, i + 5
  elseif c == "n" then return json.null, i + 4
  else
    local num = s:match("^%-?%d+%.?%d*[eE]?[%+%-]?%d*", i)
    return tonumber(num), i + #num
  end
end

function json.decode(s)
  local v = decode_value(s, 1)
  return v
end

-- --------------------------------------------------------------------------- --
-- UI builder.
-- --------------------------------------------------------------------------- --

local current_capability = ""

local function widget(spec_fn) return { to_spec = spec_fn } end

function M.Label(text, opts)
  opts = opts or {}
  return widget(function() return { kind = "label", text = text, style = opts.style or "normal" } end)
end
function M.Text(id, opts)
  opts = opts or {}
  return widget(function()
    return { kind = "text", id = id, label = opts.label or "", placeholder = opts.placeholder or "",
             multiline = opts.multiline or false, default = opts.default or "" }
  end)
end
-- A filesystem path input keyed by `id` (the chosen path comes back in the
-- method params, like Text). Typed, dropped onto, or chosen via Browse.
-- `directory` picks a folder and accepts only folders; `browse` labels the
-- button, so the module can translate it with the rest of its view.
function M.File(id, opts)
  opts = opts or {}
  return widget(function()
    -- `directory = true` is the older spelling of `accepts = "dir"`.
    local accepts = opts.accepts or "file"
    if opts.directory then accepts = "dir" end
    return { kind = "file", id = id, label = opts.label or "", placeholder = opts.placeholder or "",
             default = opts.default or "", accepts = accepts,
             browse = opts.browse or "", browse_dir = opts.browse_dir or "" }
  end)
end
function M.Select(id, options, opts)
  opts = opts or {}
  return widget(function()
    return { kind = "select", id = id, options = options, label = opts.label or "", default = opts.default or "" }
  end)
end
function M.Button(text, opts)
  opts = opts or {}
  return widget(function()
    -- `danger` is the red one, for an action that destroys something.
    local style = "default"
    if opts.danger then style = "danger" elseif opts.primary then style = "primary" end
    return { kind = "button", text = text, style = style,
             action = { capability = opts.capability or current_capability, method = opts.method or opts.calls } }
  end)
end
function M.Separator() return widget(function() return { kind = "separator" } end) end
function M.Row(children)
  return widget(function()
    local specs = {}
    for _, c in ipairs(children) do specs[#specs + 1] = c.to_spec() end
    return { kind = "row", children = specs }
  end)
end
function M.Window(title, widgets)
  return {
    to_spec = function()
      local specs = {}
      for _, w in ipairs(widgets) do specs[#specs + 1] = w.to_spec() end
      return { title = title, widgets = specs }
    end,
  }
end

-- --------------------------------------------------------------------------- --
-- Module + host.
-- --------------------------------------------------------------------------- --

local Module = {}
Module.__index = Module

function M.Module(name, capabilities)
  local self = setmetatable({}, Module)
  self.name = name
  self.capabilities = capabilities
  self._methods = {}
  self._events = {}
  self._ui = nil
  self._next_id = 0
  self.host = {
    call = function(_, cap, method, params)
      return self:_request("host.call", { capability = cap, method = method, params = params or {} })
    end,
    emit = function(_, topic, payload)
      return self:_request("host.emit", { topic = topic, payload = payload })
    end,
    subscribe = function(_, topic)
      return self:_request("host.subscribe", { subscriber = self.name, topic = topic })
    end,
    about = function(_)
      -- { os, arch, family, hostname, limen_version, base_dir }
      return self:_request("host.about", {})
    end,
    log = function(_, message)
      self:_request("host.log", tostring(message))
    end,
    -- This module's own directory on disk. Content the module fetches for
    -- itself belongs under `tools/` in here: excluded from the trust digest,
    -- removed with the module, wiped when the module updates.
    module_dir = function(_)
      local r = self:_request("host.module_dir", {})
      if type(r) == "string" and r ~= "" then return r end
      return nil
    end,
    -- Raise a desktop notification on the machine running Limen, for work the
    -- user is not watching. `urgency` is "low" | "normal" | "critical".
    -- Best-effort: a session with no notification daemon shows nothing.
    notify = function(_, title, body, urgency)
      self:_request("host.notify", { title = tostring(title), body = tostring(body or ""),
                                     urgency = urgency or "normal" })
    end,
  }
  return self
end

function Module:method(name, fn) self._methods[name] = fn; return self end
function Module:on(topic, fn) self._events[topic] = fn; return self end
function Module:ui(fn) self._ui = fn; return self end

function Module:_send(obj)
  io.write(json.encode(obj) .. "\n")
  io.flush()
end

function Module:_request(method, params)
  self._next_id = self._next_id + 1
  local id = self._next_id
  self:_send({ jsonrpc = "2.0", id = id, method = method, params = params })
  while true do
    local line = io.read("l")
    if line == nil then error("host closed the connection") end
    if line ~= "" then
      local msg = json.decode(line)
      if msg.method == nil and msg.id == id then
        if msg.error ~= nil and msg.error ~= json.null then error(msg.error.message) end
        return msg.result
      end
      self:_dispatch(msg)
    end
  end
end

function Module:_view_spec()
  if not self._ui then error("this module has no UI") end
  current_capability = self.capabilities[1] or ""
  local view = self._ui()
  if type(view) == "table" and view.to_spec then return view.to_spec() end
  return view
end

function Module:_handle(method, params)
  if method == "initialize" or method == "describe" then
    return { name = self.name, capabilities = self.capabilities }
  elseif method == "invoke" then
    local inner = params.method
    local inner_params = params.params or {}
    if inner == "ui" then return self:_view_spec() end
    local fn = self._methods[inner]
    if not fn then error("unknown method " .. tostring(inner)) end
    return fn(inner_params, self.host)
  elseif method == "shutdown" then
    return json.null
  end
  error("unknown method " .. tostring(method))
end

function Module:_dispatch(msg)
  local method = msg.method
  if method == nil then return false end
  if method == "event" and msg.id == nil then
    local p = msg.params or {}
    local handler = self._events[p.topic]
    if handler then
      local ok, err = pcall(handler, p.payload, self.host)
      if not ok then self.host:log(self.name .. ": event handler error: " .. tostring(err)) end
    end
    return false
  end
  local id = msg.id
  local ok, result = pcall(function() return self:_handle(method, msg.params or {}) end)
  if id ~= nil and id ~= json.null then
    local resp = { jsonrpc = "2.0", id = id }
    if ok then resp.result = result else resp.error = { code = -32000, message = tostring(result) } end
    self:_send(resp)
  end
  return method == "shutdown"
end

function Module:run()
  for topic in pairs(self._events) do
    local ok, err = pcall(function() self.host:subscribe(topic) end)
    if not ok then self.host:log(self.name .. ": subscribe " .. topic .. " failed: " .. tostring(err)) end
  end
  while true do
    local line = io.read("l")
    if line == nil then break end
    if line ~= "" then
      if self:_dispatch(json.decode(line)) then break end
    end
  end
end

return M
