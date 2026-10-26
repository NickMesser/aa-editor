//! Script browser and editor.
//!
//! Scripts are shown as Lua source: the edited copy in the workspace if there is one, else the decompiled original
//! (`--decompiled`), else decompiled on demand with unluac (`--unluac`, needs Java), else a read-only disassembly.
//! `toc.g` files are plain text. Saving a script writes the source to the workspace and compiles it to the client's
//! `.alb` layout into the pak overlay (the remade client and the editor read it from there; Export writes it into a
//! `game_pak`); a `toc.g` is saved as text. Then the screen is rebuilt.

use std::path::Path;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::Editor;
use crate::highlight;
use crate::loader::{self, Sources};

pub fn plugin(app: &mut App) {
    app.init_resource::<Code>().add_systems(EguiPrimaryContextPass, (open_requests, browser, editor_window).chain());
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabKind {
    Lua,
    Toc,
    /// Disassembly: no source available.
    ReadOnly,
}

pub struct Tab {
    /// Pak path (`scriptsbin/x2ui/hud/clock.alb`, `scriptsbin/x2ui/hud/toc.g`).
    pub path: String,
    pub text: String,
    /// What the file was when opened / saved (for the modified mark and Revert).
    pub saved: String,
    /// The original (decompiled or pak) text, for "Revert to original".
    pub original: String,
    pub kind: TabKind,
    /// Where the text came from.
    pub from: String,
    pub goto: Option<usize>,
    pub error: Option<(Option<usize>, String)>,
}

impl Tab {
    pub fn dirty(&self) -> bool {
        self.text != self.saved
    }
}

#[derive(Resource, Default)]
pub struct Code {
    pub tabs: Vec<Tab>,
    pub active: usize,
    filter: String,
    /// Every script and toc of the pak (built once).
    files: Vec<String>,
    only_screen: bool,
    only_edited: bool,
    find: String,
    replace: String,
    find_at: usize,
    found: Option<(usize, usize)>,
    goto_line: String,
    show_find: bool,
    new_path: String,
    new_toc: bool,
}

fn is_toc(path: &str) -> bool {
    path.ends_with("toc.g")
}

impl Code {
    fn files(&mut self) -> &[String] {
        if self.files.is_empty() {
            let mut v: Vec<String> = aa_pak::Pak::global()
                .list("scriptsbin/")
                .map(|n| {
                    let n = aa_pak::normalize(n);
                    n.strip_prefix("game/").unwrap_or(&n).to_string()
                })
                .filter(|n| n.ends_with(".alb") || n.ends_with("toc.g"))
                .collect();
            v.sort();
            v.dedup();
            self.files = v;
        }
        &self.files
    }

    /// The source of `path` as a script line number refers to it: the open tab's text, else the text it was
    /// loaded from.
    pub fn source_text(&mut self, editor: &Editor, path: &str) -> Result<String, String> {
        let i = self.open(editor, path)?;
        let t = &self.tabs[i];
        if t.kind == TabKind::ReadOnly {
            return Err("no source for this script (no decompiled copy and no unluac)".into());
        }
        Ok(t.text.clone())
    }

    /// Replace a tab's text (opening it first), mark it modified and go to `line`.
    pub fn set_text(&mut self, editor: &Editor, path: &str, text: String, line: Option<usize>) -> Result<usize, String> {
        let i = self.open(editor, path)?;
        self.tabs[i].text = text;
        self.tabs[i].goto = line;
        self.active = i;
        Ok(i)
    }

    /// Open (or switch to) the tab of `path`.
    pub fn open(&mut self, editor: &Editor, path: &str) -> Result<usize, String> {
        let path = aa_pak::normalize(path);
        let path = path.strip_prefix("game/").unwrap_or(&path).to_string();
        let path = if path.ends_with(".lua") { aa_lua::alb_path(&path) } else { path };
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.active = i;
            return Ok(i);
        }
        let tab = load_tab(&editor.sources, editor.unluac.as_deref(), &path)?;
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        Ok(self.active)
    }

    /// Save tab `i`: workspace source + compiled overlay `.alb` (or the `toc.g` text), then rebuild the screen.
    pub fn save(&mut self, editor: &mut Editor, i: usize) {
        let Some(tab) = self.tabs.get_mut(i) else { return };
        match save_tab(&editor.sources, tab) {
            Ok(msg) => {
                tab.saved = tab.text.clone();
                tab.error = None;
                editor.status(msg);
                editor.reload = true;
            }
            Err((line, msg)) => {
                if line.is_some() {
                    tab.goto = line;
                }
                editor.status(format!("not saved: {msg}"));
                tab.error = Some((line, msg));
            }
        }
    }
}

fn load_tab(sources: &Sources, unluac: Option<&Path>, path: &str) -> Result<Tab, String> {
    let pak = aa_pak::Pak::global();
    if is_toc(path) {
        let ws = sources.workspace.join(path.strip_prefix("scriptsbin/").unwrap_or(path));
        let original = pak.read_string(path).unwrap_or_default();
        let (text, from) = match std::fs::read_to_string(&ws) {
            Ok(t) => (t, "workspace".to_string()),
            Err(_) => (original.clone(), "pak".to_string()),
        };
        return Ok(Tab { path: path.into(), saved: text.clone(), text, original, kind: TabKind::Toc, from, goto: None, error: None });
    }
    let original = original_source(sources, unluac, path);
    if let Some(ws) = sources.workspace_file(path)
        && let Ok(text) = std::fs::read_to_string(&ws)
    {
        let original = original.map(|o| o.0).unwrap_or_default();
        return Ok(Tab { path: path.into(), saved: text.clone(), text, original, kind: TabKind::Lua, from: "edited (workspace)".into(), goto: None, error: None });
    }
    match original {
        Some((text, from)) => Ok(Tab { path: path.into(), saved: text.clone(), original: text.clone(), text, kind: TabKind::Lua, from, goto: None, error: None }),
        None => {
            if !pak.exists(path) {
                // a new script that is not saved yet
                return Ok(Tab { path: path.into(), text: String::new(), saved: String::new(), original: String::new(), kind: TabKind::Lua, from: "new".into(), goto: None, error: None });
            }
            let bytes = pak.read(path).map_err(|e| format!("{path}: {e}"))?;
            let chunk = aa_lua::chunk::parse(&bytes).map_err(|e| format!("{path}: {e:#}"))?;
            let mut text = String::from("-- No source: this is the bytecode disassembly (read only).\n-- Give --decompiled <dir> or --unluac <unluac.jar> (with Java) to edit it as Lua.\n\n");
            chunk.main.walk(&mut |p| text.push_str(&aa_lua::chunk::disassemble(p)));
            Ok(Tab { path: path.into(), saved: text.clone(), original: text.clone(), text, kind: TabKind::ReadOnly, from: "disassembly".into(), goto: None, error: None })
        }
    }
}

/// The decompiled original: the `--decompiled` tree, a cached unluac run, or unluac now.
fn original_source(sources: &Sources, unluac: Option<&Path>, path: &str) -> Option<(String, String)> {
    if let Some(f) = sources.decompiled_file(path)
        && let Ok(t) = std::fs::read_to_string(&f)
    {
        return Some((t, "decompiled".into()));
    }
    let rel = Sources::rel_lua(path)?;
    let cache = sources.workspace.join("../decompiled-cache").join(&rel);
    if let Ok(t) = std::fs::read_to_string(&cache) {
        return Some((t, "decompiled (unluac cache)".into()));
    }
    let jar = unluac?;
    let bytes = aa_pak::Pak::global().read(path).ok()?;
    let tmp = std::env::temp_dir().join(format!("ui-editor-{}.alb", std::process::id()));
    std::fs::write(&tmp, &bytes).ok()?;
    let out = std::process::Command::new("java").arg("-jar").arg(jar).arg(&tmp).output().ok()?;
    let _ = std::fs::remove_file(&tmp);
    if !out.status.success() {
        warn!("ui-editor: unluac {path}: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&cache, &text);
    }
    Some((text, "decompiled (unluac)".into()))
}

fn save_tab(sources: &Sources, tab: &Tab) -> Result<String, (Option<usize>, String)> {
    let io = |e: std::io::Error| (None, e.to_string());
    match tab.kind {
        TabKind::ReadOnly => Err((None, "a disassembly cannot be saved".into())),
        TabKind::Toc => {
            let rel = tab.path.strip_prefix("scriptsbin/").unwrap_or(&tab.path);
            let ws = sources.workspace.join(rel);
            std::fs::create_dir_all(ws.parent().unwrap()).map_err(io)?;
            std::fs::write(&ws, &tab.text).map_err(io)?;
            let ov = sources.overlay_file(&tab.path);
            std::fs::create_dir_all(ov.parent().unwrap()).map_err(io)?;
            std::fs::write(&ov, &tab.text).map_err(io)?;
            Ok(format!("saved {}", tab.path))
        }
        TabKind::Lua => {
            let alb = loader::compile_alb(&tab.text, &tab.path)?;
            let ws = sources.workspace_file(&tab.path).ok_or((None, "not a scriptsbin path".to_string()))?;
            std::fs::create_dir_all(ws.parent().unwrap()).map_err(io)?;
            std::fs::write(&ws, &tab.text).map_err(io)?;
            let ov = sources.overlay_file(&tab.path);
            std::fs::create_dir_all(ov.parent().unwrap()).map_err(io)?;
            std::fs::write(&ov, &alb).map_err(io)?;
            Ok(format!("saved and compiled {} ({} bytes)", tab.path, alb.len()))
        }
    }
}

/// Remove the edited copy and the overlay file: the script is the pak's again.
fn revert_to_original(sources: &Sources, tab: &mut Tab) -> String {
    let rel = tab.path.strip_prefix("scriptsbin/").unwrap_or(&tab.path).to_string();
    let ws = if tab.kind == TabKind::Toc { Some(sources.workspace.join(&rel)) } else { sources.workspace_file(&tab.path) };
    if let Some(ws) = ws {
        let _ = std::fs::remove_file(ws);
    }
    let _ = std::fs::remove_file(sources.overlay_file(&tab.path));
    tab.text = tab.original.clone();
    tab.saved = tab.original.clone();
    format!("{} reverted to the original", tab.path)
}

fn open_requests(mut editor: ResMut<Editor>, mut code: ResMut<Code>) {
    if let Some((path, line)) = editor.open_script.take() {
        match code.open(&editor, &path) {
            Ok(i) => {
                code.tabs[i].goto = line;
                editor.windows.editor = true;
            }
            Err(e) => editor.status(e),
        }
    }
}

fn browser(mut contexts: EguiContexts, mut editor: ResMut<Editor>, mut code: ResMut<Code>) -> Result {
    if editor.hidden || !editor.windows.scripts {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    let mut open = true;
    egui::Window::new("Scripts").open(&mut open).default_pos([4.0, 620.0]).default_size([300.0, 420.0]).show(&ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Find");
            ui.text_edit_singleline(&mut code.filter);
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut code.only_screen, "this screen (load order)");
            ui.checkbox(&mut code.only_edited, "edited");
        });
        let ran: Vec<(String, crate::loader::Origin)> = crate::screens::LAST.with(|l| l.borrow().log.ran.clone());
        let edited = |p: &str| editor.sources.workspace_file(p).is_some_and(|f| f.is_file())
            || (is_toc(p) && editor.sources.workspace.join(p.strip_prefix("scriptsbin/").unwrap_or(p)).is_file());
        let list: Vec<String> = if code.only_screen {
            let mut seen = std::collections::HashSet::new();
            ran.iter().map(|(p, _)| p.clone()).filter(|p| seen.insert(p.clone())).collect()
        } else {
            code.files().to_vec()
        };
        let needle = code.filter.to_lowercase();
        let rows: Vec<String> = list
            .into_iter()
            .filter(|p| needle.is_empty() || p.contains(&needle))
            .filter(|p| !code.only_edited || edited(p))
            .collect();
        ui.label(format!("{} files", rows.len()));
        let mut open_req = None;
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 2.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 70.0).show_rows(ui, row_h, rows.len(), |ui, range| {
            for p in &rows[range] {
                let short = p.trim_start_matches("scriptsbin/");
                let mark = if edited(p) { "● " } else { "" };
                if ui.selectable_label(code.tabs.get(code.active).is_some_and(|t| &t.path == p), format!("{mark}{short}")).clicked() {
                    open_req = Some(p.clone());
                }
            }
        });
        if let Some(p) = open_req {
            editor.open_script = Some((p, None));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut code.new_path).hint_text("scriptsbin/x2ui/<content>/new.lua").desired_width(190.0));
            ui.checkbox(&mut code.new_toc, "add to toc.g");
            if ui.button("New").on_hover_text("A new script; with the box ticked it is appended to its folder's toc.g so the client loads it").clicked() {
                new_script(&mut editor, &mut code);
            }
        });
    });
    if !open {
        editor.windows.scripts = false;
    }
    Ok(())
}

fn new_script(editor: &mut Editor, code: &mut Code) {
    let p = aa_pak::normalize(code.new_path.trim());
    if !p.starts_with("scriptsbin/") || !(p.ends_with(".lua") || p.ends_with(".alb")) {
        editor.status("a new script needs a path like scriptsbin/x2ui/<content>/name.lua");
        return;
    }
    let alb = aa_lua::alb_path(&p);
    match code.open(editor, &alb) {
        Ok(i) => {
            if code.tabs[i].text.is_empty() {
                code.tabs[i].text = "-- new script\n".into();
            }
        }
        Err(e) => return editor.status(e),
    }
    if code.new_toc
        && let Some((dir, file)) = p.rsplit_once('/')
    {
        let toc = format!("{dir}/toc.g");
        let lua_name = file.replace(".alb", ".lua");
        match code.open(editor, &toc) {
            Ok(i) => {
                let t = &mut code.tabs[i].text;
                if !t.lines().any(|l| l.trim() == lua_name) {
                    if !t.ends_with('\n') && !t.is_empty() {
                        t.push('\n');
                    }
                    t.push_str(&lua_name);
                    t.push('\n');
                }
                editor.status(format!("added {lua_name} to {toc}: save both (the toc after the script)"));
            }
            Err(e) => editor.status(e),
        }
        // back to the script
        let _ = code.open(editor, &alb);
    }
}

fn editor_window(mut contexts: EguiContexts, mut editor: ResMut<Editor>, mut code: ResMut<Code>) -> Result {
    if editor.hidden || !editor.windows.editor || code.tabs.is_empty() {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    let mut open = true;
    let (ctrl_s, ctrl_f, ctrl_g) = ctx.input(|i| {
        (i.modifiers.command && i.key_pressed(egui::Key::S), i.modifiers.command && i.key_pressed(egui::Key::F), i.modifiers.command && i.key_pressed(egui::Key::G))
    });
    egui::Window::new("Script editor").open(&mut open).default_pos([320.0, 520.0]).default_size([1160.0, 520.0]).show(&ctx, |ui| {
        // tabs
        let mut close = None;
        let mut activate = None;
        ui.horizontal_wrapped(|ui| {
            for (i, t) in code.tabs.iter().enumerate() {
                let name = t.path.rsplit('/').next().unwrap_or(&t.path).to_string();
                let label = if t.dirty() { format!("● {name}") } else { name };
                if ui.selectable_label(code.active == i, label).on_hover_text(&t.path).clicked() {
                    activate = Some(i);
                }
                if ui.small_button("×").clicked() {
                    close = Some(i);
                }
            }
        });
        if let Some(i) = activate {
            code.active = i;
        }
        if let Some(i) = close {
            code.tabs.remove(i);
            code.active = code.active.min(code.tabs.len().saturating_sub(1));
            return;
        }
        let i = code.active.min(code.tabs.len() - 1);
        code.active = i;
        // toolbar
        let mut save = ctrl_s;
        ui.horizontal(|ui| {
            let t = &code.tabs[i];
            ui.label(egui::RichText::new(&t.path).monospace());
            ui.weak(format!("({})", t.from));
            let editable = t.kind != TabKind::ReadOnly;
            if ui.add_enabled(editable && t.dirty(), egui::Button::new("Save (Ctrl+S)")).on_hover_text("Compile to the overlay and rebuild the screen").clicked() {
                save = true;
            }
            if ui.add_enabled(editable, egui::Button::new("Check")).on_hover_text("Compile only").clicked() {
                let r = if t.kind == TabKind::Lua { loader::compile_alb(&t.text, &t.path).map(|_| ()) } else { Ok(()) };
                match r {
                    Ok(()) => {
                        code.tabs[i].error = None;
                        editor.status("no syntax errors");
                    }
                    Err((line, msg)) => {
                        code.tabs[i].goto = line;
                        code.tabs[i].error = Some((line, msg));
                    }
                }
            }
            let t = &code.tabs[i];
            if ui.add_enabled(t.dirty(), egui::Button::new("Undo changes")).on_hover_text("Back to the last saved text").clicked() {
                let saved = code.tabs[i].saved.clone();
                code.tabs[i].text = saved;
            }
            if ui.add_enabled(editable, egui::Button::new("Revert to original")).on_hover_text("Delete the edited copy and the compiled overlay file").clicked() {
                let msg = revert_to_original(&editor.sources, &mut code.tabs[i]);
                editor.status(msg);
                editor.reload = true;
            }
            if ui.button("Find (Ctrl+F)").clicked() {
                code.show_find = !code.show_find;
            }
            ui.label("line");
            let r = ui.add(egui::TextEdit::singleline(&mut code.goto_line).desired_width(50.0));
            if ctrl_g {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                code.tabs[i].goto = code.goto_line.trim().parse().ok();
            }
        });
        if ctrl_f {
            code.show_find = true;
        }
        if code.show_find {
            find_bar(ui, &mut code, i, ctrl_f);
        }
        if let Some((line, msg)) = code.tabs[i].error.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 110, 100), &msg);
                if let Some(l) = line
                    && ui.small_button(format!("go to {l}")).clicked()
                {
                    code.tabs[i].goto = Some(l);
                }
            });
        }
        if save {
            code.save(&mut editor, i);
        }
        text_area(ui, &mut code, i);
    });
    if !open {
        editor.windows.editor = false;
    }
    Ok(())
}

fn find_bar(ui: &mut egui::Ui, code: &mut Code, i: usize, focus: bool) {
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut code.find).hint_text("find").desired_width(200.0));
        if focus {
            r.request_focus();
        }
        let enter = r.lost_focus() && ui.input(|inp| inp.key_pressed(egui::Key::Enter));
        let mut next = ui.button("next").clicked() || enter;
        let prev = ui.button("previous").clicked();
        ui.add(egui::TextEdit::singleline(&mut code.replace).hint_text("replace with").desired_width(160.0));
        if ui.button("replace all").clicked() && !code.find.is_empty() && code.tabs[i].kind != TabKind::ReadOnly {
            let n = code.tabs[i].text.matches(&code.find).count();
            code.tabs[i].text = code.tabs[i].text.replace(&code.find, &code.replace);
            ui.ctx().debug_text(format!("{n} replaced"));
        }
        let text = code.tabs[i].text.clone();
        let count = if code.find.is_empty() { 0 } else { text.matches(&code.find).count() };
        ui.weak(format!("{count} found"));
        if code.find.is_empty() {
            code.found = None;
            return;
        }
        if prev {
            let before = &text[..code.find_at.min(text.len())];
            let at = before.rfind(&code.find).or_else(|| text.rfind(&code.find));
            if let Some(at) = at {
                code.find_at = at;
                code.found = Some((at, at + code.find.len()));
                code.tabs[i].goto = Some(text[..at].matches('\n').count() + 1);
            }
            next = false;
        }
        if next {
            let from = code.found.map(|f| f.1).unwrap_or(0).min(text.len());
            let at = text[from..].find(&code.find).map(|a| a + from).or_else(|| text.find(&code.find));
            if let Some(at) = at {
                code.find_at = at;
                code.found = Some((at, at + code.find.len()));
                code.tabs[i].goto = Some(text[..at].matches('\n').count() + 1);
            }
        }
    });
}

fn text_area(ui: &mut egui::Ui, code: &mut Code, i: usize) {
    let font = egui::FontId::monospace(13.0);
    let row_h = ui.fonts_mut(|f| f.row_height(&font));
    let found = code.found;
    let tab = &mut code.tabs[i];
    let lines = tab.text.matches('\n').count() + 1;
    let toc = tab.kind == TabKind::Toc;
    let mut scroll = egui::ScrollArea::both().id_salt(("code", &tab.path)).auto_shrink([false, false]);
    if let Some(line) = tab.goto {
        scroll = scroll.vertical_scroll_offset(((line.saturating_sub(8)) as f32 * row_h).max(0.0));
    }
    let goto = tab.goto.take();
    let err_line = tab.error.as_ref().and_then(|e| e.0);
    scroll.show(ui, |ui| {
        ui.horizontal_top(|ui| {
            // line numbers
            let mut gutter = egui::text::LayoutJob::default();
            for n in 1..=lines {
                let colour = if Some(n) == err_line {
                    egui::Color32::from_rgb(255, 100, 90)
                } else if Some(n) == goto {
                    egui::Color32::from_rgb(255, 210, 90)
                } else {
                    egui::Color32::from_gray(110)
                };
                gutter.append(&format!("{n:>5}\n"), 0.0, egui::TextFormat::simple(font.clone(), colour));
            }
            ui.label(gutter);
            let font2 = font.clone();
            let mut layouter = move |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
                let job = highlight::layout(text.as_str(), toc, font2.clone(), found);
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let read_only = tab.kind == TabKind::ReadOnly;
            let mut te = egui::TextEdit::multiline(&mut tab.text)
                .id(egui::Id::new(("code-edit", &tab.path)))
                .code_editor()
                .font(font.clone())
                .lock_focus(true)
                .desired_width(f32::INFINITY)
                .desired_rows(lines.max(30))
                .layouter(&mut layouter);
            if read_only {
                te = te.interactive(false);
            }
            let out = te.show(ui);
            // put the caret on the line we jumped to
            if let Some(line) = goto {
                let at: usize = tab.text.split_inclusive('\n').take(line.saturating_sub(1)).map(|l| l.chars().count()).sum();
                let mut state = out.state.clone();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
                state.store(ui.ctx(), out.response.id);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_detection() {
        assert!(is_toc("scriptsbin/x2ui/hud/toc.g"));
        assert!(!is_toc("scriptsbin/x2ui/hud/clock.alb"));
    }
}
