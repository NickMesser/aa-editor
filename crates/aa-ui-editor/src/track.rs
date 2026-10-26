//! Where in the scripts each object was created and where its layout / look was last set.
//!
//! Before the scripts run, the native methods of every widget and drawable class are wrapped (in the class method
//! tables, so `widget.SetExtent` still exists exactly where the client has it). Each wrapper records the calling
//! script's `source:line` (the first stack frames inside `scriptsbin/`) in `AAED.created[object]` (creation) and
//! `AAED.sites[object][property]` (the last call that set the property), then calls the native.
//! `print` output goes to `AAED.console` for the editor's console.

use mlua::{Lua, Table, Value};

/// One script location: pak path (`scriptsbin/x2ui/...alb`) and line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub path: String,
    pub line: usize,
}

/// A recorded call: the method and the stack (innermost script frame first).
#[derive(Clone, Debug)]
pub struct Call {
    pub method: String,
    pub stack: Vec<Site>,
}

const TRACK_LUA: &str = r##"
return function(tables)
  local weak = { __mode = "k" }
  local SITES, CREATED = setmetatable({}, weak), setmetatable({}, weak)
  local CONSOLE = {}
  AAED = { sites = SITES, created = CREATED, console = CONSOLE }
  local getinfo, sub, select, unpack = debug.getinfo, string.sub, select, unpack
  local function stack()
    local out, l = {}, 3
    while #out < 4 and l < 24 do
      local i = getinfo(l, "Sl")
      if not i then break end
      local s = i.source
      if s and sub(s, 1, 11) == "@scriptsbin" and i.currentline and i.currentline > 0 then
        out[#out + 1] = sub(s, 2) .. ":" .. i.currentline
      end
      l = l + 1
    end
    return out
  end
  local PROPS = {
    AddAnchor = "anchor", RemoveAllAnchors = "anchor", SetExtent = "extent", SetWidth = "width",
    SetHeight = "height", Show = "show", SetText = "text", SetAlpha = "alpha", SetTexture = "texture",
    SetColor = "color", SetCoords = "coords", SetUILayer = "layer", SetTextureInfo = "texture",
    SetInset = "inset", Enable = "enable", SetTitleText = "title", SetFontSize = "fontsize",
  }
  local function pack(...) return { n = select("#", ...), ... } end
  for _, t in ipairs(tables) do
    local names = {}
    for name, f in pairs(t) do if type(f) == "function" then names[#names + 1] = name end end
    for _, name in ipairs(names) do
      local f = t[name]
      local prop = PROPS[name]
      if prop then
        t[name] = function(self, ...)
          if type(self) == "table" then
            local st = stack()
            if st[1] then
              local s = SITES[self]
              if not s then s = {}; SITES[self] = s end
              local key = prop
              if name == "AddAnchor" then
                local p = ...
                if type(p) == "string" then key = "anchor:" .. p end
              elseif name == "RemoveAllAnchors" then
                for k in pairs(s) do if sub(k, 1, 7) == "anchor:" then s[k] = nil end end
              end
              s[key] = { method = name, stack = st }
            end
          end
          return f(self, ...)
        end
      elseif sub(name, 1, 6) == "Create" then
        t[name] = function(self, ...)
          local r = pack(f(self, ...))
          local o = r[1]
          if type(o) == "table" then
            local st = stack()
            if st[1] then CREATED[o] = { method = name, stack = st } end
          end
          return unpack(r, 1, r.n)
        end
      end
    end
  end
  local tostring, concat = tostring, table.concat
  print = function(...)
    local n, parts = select("#", ...), {}
    for i = 1, n do parts[i] = tostring((select(i, ...))) end
    CONSOLE[#CONSOLE + 1] = concat(parts, "\t")
  end
end
"##;

/// Wrap the class methods. Call on a fresh runtime, before any script runs.
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let reg = aa_ui::lua::reg(lua);
    let tables = lua.create_table()?;
    let mut classes: Vec<&String> = reg.mt.keys().collect();
    classes.sort();
    let mut seen = Vec::new();
    for class in classes {
        if let Ok(Value::Table(m)) = reg.mt[class].raw_get::<Value>("__index") {
            let p = m.to_pointer();
            if !seen.contains(&p) {
                seen.push(p);
                tables.raw_push(m)?;
            }
        }
    }
    let f: mlua::Function = lua.load(TRACK_LUA).set_name("@ui-editor/track").eval()?;
    f.call::<()>(tables)
}

fn site(s: &str) -> Option<Site> {
    let (path, line) = s.rsplit_once(':')?;
    Some(Site { path: path.to_string(), line: line.parse().ok()? })
}

fn call(v: &Table) -> Option<Call> {
    let method: String = v.raw_get("method").ok()?;
    let st: Table = v.raw_get("stack").ok()?;
    let stack = st.sequence_values::<String>().filter_map(Result::ok).filter_map(|s| site(&s)).collect();
    Some(Call { method, stack })
}

/// Where object `id` was created.
pub fn created(lua: &Lua, id: aa_ui::tree::Id) -> Option<Call> {
    let obj = aa_ui::lua::object_table(lua, id).ok()??;
    let aaed: Table = lua.globals().raw_get("AAED").ok()?;
    let created: Table = aaed.raw_get("created").ok()?;
    let v: Table = created.raw_get(obj).ok()?;
    call(&v)
}

/// The recorded property sites of object `id`: (`"extent"` / `"anchor:TOPLEFT"` / ..., call).
pub fn sites(lua: &Lua, id: aa_ui::tree::Id) -> Vec<(String, Call)> {
    let mut out = Vec::new();
    let Some(obj) = aa_ui::lua::object_table(lua, id).ok().flatten() else { return out };
    let Some(aaed) = lua.globals().raw_get::<Option<Table>>("AAED").ok().flatten() else { return out };
    let Ok(sites) = aaed.raw_get::<Table>("sites") else { return out };
    let Ok(Some(s)) = sites.raw_get::<Option<Table>>(obj) else { return out };
    for (k, v) in s.pairs::<String, Table>().flatten() {
        if let Some(c) = call(&v) {
            out.push((k, c));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Lines `print`ed since the last call.
pub fn take_console(lua: &Lua) -> Vec<String> {
    let Some(aaed) = lua.globals().raw_get::<Option<Table>>("AAED").ok().flatten() else { return Vec::new() };
    let Ok(c) = aaed.raw_get::<Table>("console") else { return Vec::new() };
    let out: Vec<String> = c.sequence_values::<String>().filter_map(Result::ok).collect();
    for i in 1..=out.len() {
        let _ = c.raw_set(i, Value::Nil);
    }
    out
}
