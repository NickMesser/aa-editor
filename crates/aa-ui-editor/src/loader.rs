//! Runs a screen's scripts the way the client does (init script, then [`aa_ui::Stage::scripts`] in order), but each
//! file comes from the best source there is:
//!
//! 1. the editor workspace (`<workspace>/<rel>.lua`, a script the user edited),
//! 2. in source mode, the decompiled original (`<decompiled>/<rel>.lua`), so line numbers in errors and call sites
//!    match the text the script editor shows,
//! 3. else the `.alb` from the pak (or the pak overlay, where saved scripts are compiled to).
//!
//! `Script.ReloadScript` / `LoadScript` (the `*_view.lua` files the scripts pull in) go through the same choice.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use aa_ui::UiRuntime;
use mlua::Lua;

/// Where scripts and their sources live.
#[derive(Clone, Debug)]
pub struct Sources {
    /// Decompiled originals, laid out like `scriptsbin/` (`x2ui/hud/clock.lua`).
    pub decompiled: Option<PathBuf>,
    /// Edited sources, same layout.
    pub workspace: PathBuf,
    /// The pak overlay folder (`scriptsbin/x2ui/hud/clock.alb` compiled scripts).
    pub overlay: PathBuf,
    /// Run decompiled sources instead of the `.alb` when there is no edited one.
    pub source_mode: bool,
}

/// Which text a script ran from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Workspace,
    Decompiled,
    Alb,
}

impl Sources {
    /// `scriptsbin/x2ui/hud/clock.alb` -> `x2ui/hud/clock.lua`.
    pub fn rel_lua(pak_path: &str) -> Option<String> {
        let n = aa_pak::normalize(pak_path);
        let n = n.strip_prefix("game/").unwrap_or(&n);
        let rest = n.strip_prefix("scriptsbin/")?;
        let base = rest.strip_suffix(".alb").or_else(|| rest.strip_suffix(".lua")).unwrap_or(rest);
        Some(format!("{base}.lua"))
    }

    pub fn workspace_file(&self, pak_path: &str) -> Option<PathBuf> {
        Some(self.workspace.join(Self::rel_lua(pak_path)?))
    }

    pub fn decompiled_file(&self, pak_path: &str) -> Option<PathBuf> {
        Some(self.decompiled.as_ref()?.join(Self::rel_lua(pak_path)?))
    }

    /// The overlay file a compiled script (or a `toc.g`) is saved to.
    pub fn overlay_file(&self, pak_path: &str) -> PathBuf {
        let n = aa_pak::normalize(pak_path);
        let n = n.strip_prefix("game/").unwrap_or(&n).to_string();
        self.overlay.join(n)
    }

    /// The text a script should run from, if not its `.alb`.
    pub fn source_for(&self, pak_path: &str) -> Option<(String, Origin)> {
        if let Some(f) = self.workspace_file(pak_path)
            && let Ok(t) = std::fs::read_to_string(&f)
        {
            return Some((t, Origin::Workspace));
        }
        if self.source_mode
            && let Some(f) = self.decompiled_file(pak_path)
            && let Ok(t) = std::fs::read_to_string(&f)
        {
            return Some((t, Origin::Decompiled));
        }
        None
    }
}

/// What the loader did, for the UI (which files ran from which text).
#[derive(Default, Debug)]
pub struct LoadLog {
    pub ran: Vec<(String, Origin)>,
}

/// Load one script (compile, not run) from the chosen text. `pak_path` is a `scriptsbin/...` path (`.lua` or `.alb`).
pub fn load(lua: &Lua, sources: &Sources, pak_path: &str) -> anyhow::Result<(mlua::Function, Origin)> {
    let alb = aa_lua::alb_path(pak_path);
    let name = aa_pak::normalize(&alb);
    match sources.source_for(&alb) {
        Some((text, origin)) => {
            let f = lua
                .load(text.as_str())
                .set_name(format!("@{name}"))
                .set_mode(mlua::chunk::ChunkMode::Text)
                .into_function()
                .map_err(|e| anyhow::anyhow!("compiling {name}: {e}"))?;
            Ok((f, origin))
        }
        None => Ok((aa_lua::load_alb(lua, &alb)?, Origin::Alb)),
    }
}

/// Run a list of scripts in order, carrying on past failures (pushed to `rt.errors`).
pub fn run_files(rt: &mut UiRuntime, sources: &Sources, files: &[String], log: &Rc<RefCell<LoadLog>>) {
    for path in files {
        if let Err(e) = run_one(&rt.lua, sources, path, log) {
            bevy::log::warn!("ui-editor: script {path}: {e:#}");
            rt.errors.push((path.clone(), format!("{e:#}")));
        }
    }
    aa_ui::lua::flush(&rt.lua);
}

fn run_one(lua: &Lua, sources: &Sources, path: &str, log: &Rc<RefCell<LoadLog>>) -> anyhow::Result<()> {
    let (f, origin) = load(lua, sources, path)?;
    log.borrow_mut().ran.push((aa_pak::normalize(&aa_lua::alb_path(path)), origin));
    f.call::<()>(()).map_err(|e| anyhow::anyhow!("running {path}: {e}"))
}

/// Replace CryEngine's `Script.ReloadScript` / `LoadScript` / `UnloadScript` so pulled-in files use [`load`] too.
pub fn install_script_table(lua: &Lua, sources: &Sources, log: &Rc<RefCell<LoadLog>>) -> mlua::Result<()> {
    let loaded: Rc<RefCell<HashSet<String>>> = Rc::default();
    let script: mlua::Table = match lua.globals().get::<Option<mlua::Table>>("Script")? {
        Some(t) => t,
        None => lua.create_table()?,
    };
    let exec = {
        let (sources, log, loaded) = (sources.clone(), log.clone(), loaded.clone());
        Rc::new(move |lua: &Lua, path: &str| -> bool {
            let p = aa_lua::script_path(path);
            loaded.borrow_mut().insert(p.clone());
            match run_one(lua, &sources, &p, &log) {
                Ok(()) => true,
                Err(e) => {
                    bevy::log::warn!("[Lua Error] {e:#}");
                    false
                }
            }
        })
    };
    {
        let exec = exec.clone();
        script.set("ReloadScript", lua.create_function(move |lua, path: String| Ok(exec(lua, &path)))?)?;
    }
    {
        let (exec, loaded) = (exec.clone(), loaded.clone());
        script.set(
            "LoadScript",
            lua.create_function(move |lua, (path, reload, _raise): (String, Option<bool>, Option<bool>)| {
                let done = loaded.borrow().contains(&aa_lua::script_path(&path));
                Ok(if done && !reload.unwrap_or(false) { true } else { exec(lua, &path) })
            })?,
        )?;
    }
    script.set(
        "UnloadScript",
        lua.create_function(move |_, path: String| {
            loaded.borrow_mut().remove(&aa_lua::script_path(&path));
            Ok(())
        })?,
    )?;
    lua.globals().set("Script", script)
}

/// Compile Lua source to the client's `.alb` layout (32-bit, `float` numbers), as the original `luac` wrote them.
/// On a syntax error returns `(line, message)`.
pub fn compile_alb(text: &str, pak_path: &str) -> Result<Vec<u8>, (Option<usize>, String)> {
    let lua = Lua::new();
    let name = aa_pak::normalize(&aa_lua::alb_path(pak_path));
    let f = lua.load(text).set_name(format!("@{name}")).set_mode(mlua::chunk::ChunkMode::Text).into_function().map_err(|e| {
        let msg = e.to_string();
        (syntax_line(&msg), msg)
    })?;
    let host = f.dump(false);
    let chunk = aa_lua::chunk::parse(&host).map_err(|e| (None, format!("{e:#}")))?;
    aa_lua::chunk::write(&chunk, aa_lua::chunk::Format::ALB).map_err(|e| (None, format!("{e:#}")))
}

/// The line of a Lua error message (`[string "..."]:12:` or `file.alb:12:`).
pub fn syntax_line(msg: &str) -> Option<usize> {
    for (i, _) in msg.match_indices(':') {
        let rest = &msg[i + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && rest[digits.len()..].starts_with(':') {
            return digits.parse().ok();
        }
    }
    None
}

/// A `scriptsbin/...:LINE:` reference in an error or traceback: (pak path, line).
pub fn script_ref(msg: &str) -> Option<(String, usize)> {
    let start = msg.find("scriptsbin/")?;
    let rest = &msg[start..];
    let end = rest.find(|c: char| c == ':' || c.is_whitespace() || c == '"' || c == ']')?;
    let path = &rest[..end];
    let after = &rest[end..];
    let line = after.strip_prefix(':').and_then(|a| a.split(|c: char| !c.is_ascii_digit()).next()).and_then(|d| d.parse().ok())?;
    Some((path.to_string(), line))
}

/// `true` when `dir` looks like a decompiled `scriptsbin` tree.
pub fn is_decompiled_dir(dir: &Path) -> bool {
    dir.join("x2ui").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(Sources::rel_lua("scriptsbin/x2ui/hud/clock.alb").as_deref(), Some("x2ui/hud/clock.lua"));
        assert_eq!(Sources::rel_lua("game/scriptsbin/x2ui/hud/clock.lua").as_deref(), Some("x2ui/hud/clock.lua"));
        assert_eq!(Sources::rel_lua("ui/x.dds"), None);
    }

    #[test]
    fn error_lines() {
        assert_eq!(syntax_line("[string \"@scriptsbin/x2ui/a.alb\"]:12: '=' expected near 'x'"), Some(12));
        assert_eq!(
            script_ref("runtime error: scriptsbin/x2ui/hud/clock.alb:40: attempt to index nil"),
            Some(("scriptsbin/x2ui/hud/clock.alb".into(), 40))
        );
    }

    #[test]
    fn compiles_to_the_alb_layout_and_loads_back() {
        let src = "local t = { 0.7, 3, 'x' }\nfunction F(a) return a * t[1] end\n";
        let alb = compile_alb(src, "scriptsbin/x2ui/test.lua").unwrap();
        assert_eq!(&alb[..12], &[0x1b, b'L', b'u', b'a', 0x51, 0, 1, 4, 4, 4, 4, 0]);
        let lua = aa_lua::new_lua().unwrap();
        aa_lua::load_alb_bytes(&lua, &alb, "scriptsbin/x2ui/test.alb").unwrap().call::<()>(()).unwrap();
        let r: f64 = lua.load("return F(2)").eval().unwrap();
        assert!((r - 1.4).abs() < 1e-6, "{r}");
        let (line, _) = compile_alb("local x = = 1", "scriptsbin/x2ui/test.lua").unwrap_err();
        assert_eq!(line, Some(1));
    }

    /// Every decompiled original compiles to the `.alb` layout and loads back (needs the decompiled tree).
    #[test]
    fn all_decompiled_scripts_compile() {
        let dir = std::path::Path::new("D:/AABevy1.2/data/extract/aa-lua/decompiled");
        if !is_decompiled_dir(dir) {
            return;
        }
        let lua = aa_lua::new_lua().unwrap();
        let (mut n, mut failed) = (0, Vec::new());
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "lua") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    let name = format!("scriptsbin/{}", p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
                    n += 1;
                    match compile_alb(&text, &name) {
                        Ok(alb) => {
                            if let Err(e) = aa_lua::load_alb_bytes(&lua, &alb, &name) {
                                failed.push(format!("{name}: load: {e:#}"));
                            }
                        }
                        Err((_, e)) => failed.push(format!("{name}: {e}")),
                    }
                }
            }
        }
        assert!(n > 900, "{n} scripts");
        assert!(failed.is_empty(), "{} of {n} failed:\n{}", failed.len(), failed.join("\n"));
    }
}
