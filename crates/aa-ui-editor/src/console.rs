//! Script errors (load errors and runtime errors; click to open the line), `print` output, and a Lua prompt that runs
//! in the screen's Lua state. Also the start-up flags that drive the editor unattended (`--open`, `--select`, `--lua`).

use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::inspect::Inspect;
use crate::{Editor, arg, loader, track};

pub fn plugin(app: &mut App) {
    app.init_resource::<Console>().add_systems(Update, (collect, scripted)).add_systems(EguiPrimaryContextPass, window);
}

#[derive(Resource, Default)]
pub struct Console {
    pub lines: Vec<(bool, String)>,
    input: String,
    history: Vec<String>,
    tab: usize,
}

fn collect(rt: Option<NonSend<UiRuntime>>, mut console: ResMut<Console>) {
    let Some(rt) = rt else { return };
    for l in track::take_console(&rt.lua) {
        console.lines.push((false, l));
    }
    let n = console.lines.len();
    if n > 5000 {
        console.lines.drain(..n - 4000);
    }
}

/// `--open ACTION`, `--select <Lua expression>`, `--lua CODE`: once, a moment after the screen is built.
fn scripted(rt: Option<NonSend<UiRuntime>>, mut editor: ResMut<Editor>, mut st: ResMut<Inspect>, time: Res<Time>, mut done: Local<bool>, mut t0: Local<Option<f32>>) {
    // `done` survives rebuilds: after --write-into-script saves and rebuilds, the checks do not run again
    let Some(rt) = rt else { return };
    if *done {
        return;
    }
    let start = *t0.get_or_insert(time.elapsed_secs());
    if time.elapsed_secs() - start < 1.0 {
        return;
    }
    *done = true;
    if let Some(action) = arg("--open")
        && let Err(e) = rt.hotkey(&action)
    {
        editor.status(format!("--open {action}: {e}"));
    }
    if let Some(code) = arg("--lua")
        && let Err(e) = rt.exec("@--lua", &code)
    {
        editor.status(format!("--lua: {e}"));
    }
    if let Some(expr) = arg("--select") {
        // a Lua expression (`bag.titleBar`), else a widget name
        let by_expr = rt.lua.load(expr.as_str()).set_name("=--select").eval::<mlua::Value>().ok().and_then(|v| aa_ui::lua::value_id(&rt.lua, &v));
        let id = by_expr.or_else(|| aa_ui::lua::ui(&rt.lua).tree.find(&expr));
        match id {
            Some(id) => st.select(Some(id)),
            None => editor.status(format!("--select {expr}: no such widget")),
        }
    }
    // checks: change the selection like the inspector does, then write the change into its script
    if let Some(id) = st.selected {
        let v = |name: &str| arg(name).map(|s| s.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<f32>>());
        if let Some(e) = v("--set-extent").filter(|e| e.len() == 2) {
            match crate::inspect::call(&rt.lua, id, "SetExtent", (e[0], e[1])) {
                Ok(()) => st.record(id, crate::patch::Edit::Extent(e[0], e[1])),
                Err(err) => editor.status(format!("--set-extent: {err}")),
            }
        }
        if let Some(d) = v("--move").filter(|d| d.len() == 2) {
            let anchors = aa_ui::lua::ui(&rt.lua).tree.get(id).anchors.clone();
            for a in anchors {
                let target = aa_ui::lua::object_table(&rt.lua, a.target).ok().flatten();
                let r = crate::inspect::call(&rt.lua, id, "AddAnchor", (a.point.name(), target, a.rel.name(), a.x + d[0], a.y + d[1]));
                match r {
                    Ok(()) => st.record(id, crate::patch::Edit::Anchor { point: a.point.name().into(), rel: a.rel.name().into(), x: a.x + d[0], y: a.y + d[1] }),
                    Err(err) => editor.status(format!("--move: {err}")),
                }
            }
        }
        if let Some(t) = arg("--set-text") {
            match crate::inspect::call(&rt.lua, id, "SetText", t.clone()) {
                Ok(()) => st.record(id, crate::patch::Edit::Text(t)),
                Err(err) => editor.status(format!("--set-text: {err}")),
            }
        }
        if crate::flag("--write-into-script") {
            st.auto_write = true;
        }
    }
}

fn error_row(ui: &mut egui::Ui, editor: &mut Editor, text: &str) {
    ui.horizontal_wrapped(|ui| {
        if let Some((path, line)) = loader::script_ref(text)
            && ui.small_button("open").on_hover_text(format!("{path}:{line}")).clicked()
        {
            editor.open_script = Some((path, Some(line)));
        }
        ui.colored_label(egui::Color32::from_rgb(255, 140, 120), egui::RichText::new(text).monospace());
    });
}

fn window(mut contexts: EguiContexts, mut editor: ResMut<Editor>, mut console: ResMut<Console>, rt: Option<NonSend<UiRuntime>>) -> Result {
    if editor.hidden || !editor.windows.console {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    let mut open = true;
    let load_errors: Vec<(String, String)> = rt.as_ref().map(|r| r.errors.clone()).unwrap_or_default();
    let runtime = aa_dev::capture::lua_errors();
    let nlines = console.lines.len();
    egui::Window::new("Console").open(&mut open).default_pos([1500.0, 700.0]).default_size([410.0, 330.0]).show(&ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut console.tab, 0, format!("Errors ({} load, {} runtime)", load_errors.len(), runtime.len()));
            ui.selectable_value(&mut console.tab, 1, format!("Output ({nlines})"));
            if ui.small_button("clear output").clicked() {
                console.lines.clear();
            }
            if let Some(rt) = rt.as_ref() {
                let stubs = rt.stub_calls();
                ui.weak(format!("{} unimplemented natives called", stubs.len())).on_hover_text(
                    stubs.iter().take(40).map(|(n, c)| format!("{n} x{c}")).collect::<Vec<_>>().join("\n"),
                );
            }
        });
        ui.separator();
        let h = ui.available_height() - 30.0;
        egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            if console.tab == 0 {
                if load_errors.is_empty() && runtime.is_empty() {
                    ui.weak("no script errors");
                }
                for (path, e) in &load_errors {
                    error_row(ui, &mut editor, &format!("{path}: {e}"));
                }
                for e in &runtime {
                    error_row(ui, &mut editor, e);
                }
            } else {
                for (err, l) in &console.lines {
                    if *err {
                        error_row(ui, &mut editor, l);
                    } else {
                        ui.monospace(l);
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Lua>");
            let r = ui.add(egui::TextEdit::singleline(&mut console.input).code_editor().desired_width(f32::INFINITY).hint_text("runs in the screen's Lua state, e.g. bag:Show(true)  or  =UIParent:GetScreenWidth()"));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let code = std::mem::take(&mut console.input);
                if let Some(rt) = rt.as_ref() {
                    console.tab = 1;
                    console.lines.push((false, format!("> {code}")));
                    let out = run(&rt.lua, &code);
                    for l in track::take_console(&rt.lua) {
                        console.lines.push((false, l));
                    }
                    match out {
                        Ok(Some(v)) => console.lines.push((false, v)),
                        Ok(None) => {}
                        Err(e) => console.lines.push((true, e)),
                    }
                    aa_ui::lua::flush(&rt.lua);
                }
                console.history.push(code);
                r.request_focus();
            }
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::ArrowUp))
                && let Some(last) = console.history.last()
            {
                console.input = last.clone();
            }
        });
    });
    if !open {
        editor.windows.console = false;
    }
    Ok(())
}

/// Run `code`; an expression (or `=expr`) shows its values.
fn run(lua: &mlua::Lua, code: &str) -> Result<Option<String>, String> {
    let code = code.trim();
    let expr = code.strip_prefix('=').unwrap_or(code);
    let show = |vals: mlua::MultiValue| -> Option<String> {
        if vals.is_empty() {
            return None;
        }
        let tostring: mlua::Function = lua.globals().get("tostring").ok()?;
        Some(vals.into_iter().map(|v| tostring.call::<String>(v).unwrap_or_default()).collect::<Vec<_>>().join("\t"))
    };
    if let Ok(f) = lua.load(format!("return {expr}")).set_name("=console").into_function() {
        return f.call::<mlua::MultiValue>(()).map(show).map_err(|e| e.to_string());
    }
    lua.load(code).set_name("=console").call::<mlua::MultiValue>(()).map(show).map_err(|e| e.to_string())
}
