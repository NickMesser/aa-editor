//! The menu bar, the widget tree, picking on screen and the inspector (live edits, drag to move / resize, write the
//! change into the script).

use std::collections::{BTreeMap, HashSet};

use aa_ui::UiRuntime;
use aa_ui::tree::{Id, Kind, Point, Rect};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use mlua::IntoLuaMulti;

use crate::code::Code;
use crate::patch::{self, Edit};
use crate::screens::Screen;
use crate::track;
use crate::{Editor, screens};

pub fn plugin(app: &mut App) {
    app.init_resource::<Inspect>().add_systems(EguiPrimaryContextPass, panels);
}

const POINTS: [Point; 9] = [
    Point::TopLeft,
    Point::Top,
    Point::TopRight,
    Point::Left,
    Point::Center,
    Point::Right,
    Point::BottomLeft,
    Point::Bottom,
    Point::BottomRight,
];

/// Key binding actions that open in-game windows.
const ACTIONS: [(&str, &str); 11] = [
    ("toggle_bag", "Bag (I)"),
    ("toggle_character", "Character (C)"),
    ("toggle_spellbook", "Skills (K)"),
    ("toggle_quest", "Quests (L)"),
    ("toggle_worldmap", "World map (M)"),
    ("toggle_achievement", "Achievements"),
    ("toggle_ranking", "Ranking"),
    ("toggle_craft_book", "Craft book"),
    ("toggle_friend", "Friends"),
    ("toggle_option", "Options"),
    ("open_config", "System menu (Esc)"),
];

#[derive(Resource, Default)]
pub struct Inspect {
    pub selected: Option<Id>,
    pub hover: Option<Id>,
    /// Widgets under the pointer at the last pick, topmost first.
    pub stack: Vec<Id>,
    pub filter: String,
    pub show_hidden: bool,
    pub show_drawables: bool,
    /// Expand the tree down to the selection once.
    reveal: bool,
    drag: Option<Drag>,
    /// The last value of each property changed in the inspector, per object (not yet written into a script).
    pub edits: BTreeMap<Id, BTreeMap<String, Edit>>,
    /// Patches waiting for confirmation.
    preview: Option<Vec<Planned>>,
    paths: Option<(Id, Vec<String>)>,
    text_buf: Option<(Id, String)>,
    texture_buf: Option<(Id, String)>,
    target_buf: String,
    contents: Vec<(String, i64)>,
    /// `--write-into-script`: plan, apply and save the selection's changes without asking.
    pub auto_write: bool,
}

impl Inspect {
    pub fn clear(&mut self) {
        let keep = (self.filter.clone(), self.show_hidden, self.show_drawables);
        *self = Inspect { filter: keep.0, show_hidden: keep.1, show_drawables: keep.2, ..default() };
    }

    pub fn select(&mut self, id: Option<Id>) {
        if self.selected != id {
            self.selected = id;
            self.reveal = true;
            self.text_buf = None;
            self.texture_buf = None;
        }
    }

    pub fn record(&mut self, id: Id, edit: Edit) {
        self.edits.entry(id).or_default().insert(edit.key(), edit);
    }
}

struct Drag {
    id: Id,
    start: egui::Pos2,
    resize: bool,
    anchors: Vec<aa_ui::tree::Anchor>,
    extent: (f32, f32),
}

/// A patch ready to apply.
struct Planned {
    path: String,
    result: Result<patch::Patched, String>,
    id: Id,
    key: String,
}

/// Call a method of an object table (`obj:Method(args)`).
pub fn call(lua: &mlua::Lua, id: Id, method: &str, args: impl IntoLuaMulti) -> mlua::Result<()> {
    let t = aa_ui::lua::object_table(lua, id)?.ok_or_else(|| mlua::Error::runtime("object has no table"))?;
    let f: mlua::Function = t.get(method)?;
    let mut mv = args.into_lua_multi(lua)?;
    mv.push_front(mlua::Value::Table(t));
    f.call::<()>(mv)?;
    aa_ui::lua::flush(lua);
    Ok(())
}

fn label_of(lua: &mlua::Lua, id: Id) -> String {
    let g = aa_ui::lua::ui(lua);
    let o = g.tree.get(id);
    let class = match &o.kind {
        Kind::Widget(w) => w.class.clone(),
        Kind::Drawable(d) => d.class.to_string(),
    };
    if o.name.is_empty() { format!("<{class}> #{id}") } else { format!("{} <{class}>", o.name) }
}

/// Topmost visible widgets containing UI point `(x, y)`, first those that draw something.
fn pick_stack(lua: &mlua::Lua, x: f32, y: f32) -> Vec<Id> {
    let g = aa_ui::lua::ui(lua);
    let tree = &g.tree;
    let mut drawing = Vec::new();
    let mut others = Vec::new();
    for id in tree.paint_order().into_iter().rev() {
        let o = tree.get(id);
        let Kind::Widget(w) = &o.kind else { continue };
        if !o.alive || !tree.visible(id) {
            continue;
        }
        let r = tree.rect(id);
        if r.w <= 0.0 || r.h <= 0.0 || !r.contains(x, y) {
            continue;
        }
        let draws = w.text.as_ref().is_some_and(|t| !t.text.is_empty())
            || w.drawables.iter().any(|&d| tree.get(d).shown)
            || w.ty.is_button_like();
        if draws { drawing.push(id) } else { others.push(id) }
        if drawing.len() + others.len() >= 24 {
            break;
        }
    }
    drawing.extend(others);
    drawing
}

#[allow(clippy::too_many_arguments)]
fn panels(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut st: ResMut<Inspect>,
    mut code: ResMut<Code>,
    rt: Option<NonSend<UiRuntime>>,
    stats: Res<aa_ui::UiStats>,
    keys: Res<ButtonInput<KeyCode>>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    mut fonts: Local<bool>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    if !*fonts {
        *fonts = true;
        install_fonts(&ctx);
    }
    editor.egui_pointer = ctx.egui_wants_pointer_input() || ctx.is_pointer_over_egui() || ctx.egui_is_using_pointer();
    editor.egui_keyboard = ctx.egui_wants_keyboard_input();
    if editor.hidden {
        return Ok(());
    }
    let fps = diagnostics.get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()).unwrap_or(0.0);
    menu_bar(&ctx, &mut editor, &mut st, rt.as_deref(), &stats, fps);
    let Some(rt) = rt else { return Ok(()) };
    let lua = &rt.lua;
    // a selection whose object died (screen rebuilt, window removed)
    if let Some(id) = st.selected {
        let alive = { let g = aa_ui::lua::ui(lua); (id as usize) < g.tree.objects.len() && g.tree.get(id).alive };
        if !alive {
            st.select(None);
        }
    }
    overlay(&ctx, &mut editor, &mut st, &rt, &keys);
    if editor.windows.tree {
        tree_window(&ctx, &mut editor, &mut st, lua);
    }
    if editor.windows.inspector {
        inspector(&ctx, &mut editor, &mut st, &mut code, lua);
    }
    if std::mem::take(&mut st.auto_write)
        && let Some(id) = st.selected
    {
        let planned = plan(lua, &mut code, &editor, &st, id);
        for p in &planned {
            match &p.result {
                Ok(r) => info!("ui-editor: write into script: {} {}", p.path, r.what),
                Err(e) => warn!("ui-editor: write into script: {} {}: {e}", p.path, p.key),
            }
        }
        st.preview = Some(planned);
        preview_apply(&mut editor, &mut st, &mut code, true);
    }
    preview_window(&ctx, &mut editor, &mut st, &mut code);
    Ok(())
}

/// Korean texts in the scripts need a Hangul font: Windows' Malgun Gothic as a fallback, when present.
fn install_fonts(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read("C:/Windows/Fonts/malgun.ttf") else { return };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("hangul".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("hangul".into());
    }
    ctx.set_fonts(fonts);
}

fn menu_bar(ctx: &egui::Context, editor: &mut Editor, st: &mut Inspect, rt: Option<&UiRuntime>, stats: &aa_ui::UiStats, fps: f64) {
    egui::Area::new("menu".into()).anchor(egui::Align2::LEFT_TOP, [0.0, 0.0]).show(ctx, |ui| {
        egui::Frame::window(ui.style()).corner_radius(0.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("UI editor");
                ui.separator();
                let before = editor.spec.screen;
                egui::ComboBox::from_id_salt("screen").selected_text(editor.spec.screen.label()).show_ui(ui, |ui| {
                    for s in Screen::ALL {
                        ui.selectable_value(&mut editor.spec.screen, s, s.label());
                    }
                });
                if editor.spec.screen != before {
                    editor.reload = true;
                }
                match editor.spec.screen {
                    Screen::CharacterCreate => {
                        let old = (editor.spec.race.clone(), editor.spec.gender.clone(), editor.spec.step);
                        egui::ComboBox::from_id_salt("race").selected_text(editor.spec.race.clone()).show_ui(ui, |ui| {
                            for r in ["nuian", "elf", "ferre", "hariharan"] {
                                ui.selectable_value(&mut editor.spec.race, r.to_string(), r);
                            }
                        });
                        egui::ComboBox::from_id_salt("gender").selected_text(editor.spec.gender.clone()).show_ui(ui, |ui| {
                            for g in ["male", "female"] {
                                ui.selectable_value(&mut editor.spec.gender, g.to_string(), g);
                            }
                        });
                        ui.add(egui::Slider::new(&mut editor.spec.step, 1..=3).text("step"));
                        if old != (editor.spec.race.clone(), editor.spec.gender.clone(), editor.spec.step) {
                            editor.reload = true;
                        }
                    }
                    Screen::CharacterSelect => {
                        if ui.checkbox(&mut editor.spec.with_character, "with a character").changed() {
                            editor.reload = true;
                        }
                    }
                    Screen::World => {
                        ui.menu_button("Open window", |ui| {
                            for (action, label) in ACTIONS {
                                if ui.button(label).clicked()
                                    && let Some(rt) = rt
                                {
                                    match rt.hotkey(action) {
                                        Ok(true) => editor.status(format!("{label} toggled")),
                                        Ok(false) => editor.status(format!("{action} is not a UI window here")),
                                        Err(e) => editor.status(format!("{action}: {e}")),
                                    }
                                }
                            }
                            ui.separator();
                            ui.menu_button("Any content (UIC_*)", |ui| {
                                if st.contents.is_empty()
                                    && let Some(rt) = rt
                                {
                                    st.contents = contents(&rt.lua);
                                }
                                egui::ScrollArea::vertical().max_height(500.0).show(ui, |ui| {
                                    for (name, id) in st.contents.clone() {
                                        let shown = rt.is_some_and(|r| r.content_visible(id));
                                        if ui.selectable_label(shown, &name).clicked()
                                            && let Some(rt) = rt
                                            && let Err(e) = rt.toggle_content(id)
                                        {
                                            editor.status(format!("{name}: {e}"));
                                        }
                                    }
                                });
                            });
                        });
                    }
                    _ => {}
                }
                if ui.button("Rebuild (F5)").on_hover_text("Run the screen's scripts again in a fresh Lua state (picks up saved scripts)").clicked() {
                    editor.reload = true;
                }
                ui.toggle_value(&mut editor.pick, "Pick (F2)").on_hover_text("The pointer selects widgets instead of clicking them; drag to move, drag the corner to resize. Alt+click picks at any time.");
                let can_source = editor.sources.decompiled.is_some();
                let mut sm = editor.sources.source_mode;
                if ui
                    .add_enabled(can_source, egui::Checkbox::new(&mut sm, "run decompiled sources"))
                    .on_hover_text("Run every script from its decompiled Lua (line numbers match the script editor) instead of the .alb bytecode. Edited scripts always run from their source.")
                    .changed()
                {
                    editor.sources.source_mode = sm;
                    editor.reload = true;
                }
                ui.menu_button("Windows", |ui| {
                    let w = &mut editor.windows;
                    ui.checkbox(&mut w.tree, "Widget tree");
                    ui.checkbox(&mut w.inspector, "Inspector");
                    ui.checkbox(&mut w.scripts, "Script browser");
                    ui.checkbox(&mut w.editor, "Script editor");
                    ui.checkbox(&mut w.console, "Console");
                    ui.checkbox(&mut w.export, "Export to game_pak");
                    ui.separator();
                    ui.label("F1 hides all panels");
                });
                ui.separator();
                let (objects, errors) = rt.map(|rt| (aa_ui::lua::ui(&rt.lua).tree.objects.len(), rt.errors.len())).unwrap_or((0, 0));
                let built = screens::LAST.with(|l| l.borrow().ms);
                ui.label(format!("{objects} objects · {errors} load errors · built in {built:.0} ms · {} quads · {fps:.0} fps", stats.quads));
                if !editor.status.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&editor.status).italics());
                }
            });
        });
    });
}

/// `UIC_*` content ids the scripts registered.
fn contents(lua: &mlua::Lua) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = lua
        .globals()
        .pairs::<mlua::Value, mlua::Value>()
        .flatten()
        .filter_map(|(k, v)| {
            let k = k.as_string()?.to_str().ok()?.to_string();
            let v = v.as_integer().or_else(|| v.as_number().map(|n| n as i64))?;
            k.starts_with("UIC_").then_some((k, v))
        })
        .collect();
    out.sort();
    out
}

fn to_points(r: &Rect, scale: f32, ppp: f32) -> egui::Rect {
    let s = scale / ppp;
    egui::Rect::from_min_size(egui::pos2(r.x * s, r.y * s), egui::vec2(r.w * s, r.h * s))
}

/// Selection / hover outlines, picking and dragging on the screen itself.
fn overlay(ctx: &egui::Context, editor: &mut Editor, st: &mut Inspect, rt: &UiRuntime, keys: &ButtonInput<KeyCode>) {
    let lua = &rt.lua;
    let scale = rt.ui_scale();
    let ppp = ctx.pixels_per_point();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, "ui-editor overlay".into()));
    let rect_of = |id: Id| aa_ui::lua::ui(lua).tree.rect(id);
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let on_screen = pointer.filter(|_| !editor.egui_pointer);
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let picking = editor.pick || alt;
    // hover
    st.hover = None;
    if picking && let Some(p) = on_screen {
        let s = ppp / scale;
        let stack = pick_stack(lua, p.x * s, p.y * s);
        st.hover = stack.first().copied();
        let (pressed, released) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released()));
        if pressed {
            // grab the selection (move, or resize at its bottom-right corner) or pick
            let grab = st.selected.filter(|&sel| {
                let r = to_points(&rect_of(sel), scale, ppp);
                r.expand(2.0).contains(p)
            });
            match grab {
                Some(sel) if editor.pick => {
                    let r = to_points(&rect_of(sel), scale, ppp);
                    let resize = (r.max - p).length() < 10.0;
                    let g = aa_ui::lua::ui(lua);
                    let o = g.tree.get(sel);
                    st.drag = Some(Drag { id: sel, start: p, resize, anchors: o.anchors.clone(), extent: (o.width, o.height) });
                }
                _ => {
                    st.stack = stack.clone();
                    st.select(stack.first().copied());
                }
            }
        }
        if released && let Some(d) = st.drag.take() {
            let o = { let g = aa_ui::lua::ui(lua); let o = g.tree.get(d.id); (o.anchors.clone(), o.width, o.height) };
            if d.resize {
                if (o.1, o.2) != d.extent {
                    st.record(d.id, Edit::Extent(o.1, o.2));
                }
            } else if o.0 != d.anchors {
                for a in &o.0 {
                    st.record(d.id, Edit::Anchor { point: a.point.name().into(), rel: a.rel.name().into(), x: a.x, y: a.y });
                }
            }
        }
    }
    if let (Some(d), Some(p)) = (st.drag.as_ref(), pointer) {
        let delta = (p - d.start) * ppp / scale;
        let (dx, dy) = (delta.x.round(), delta.y.round());
        if d.resize {
            let (w, h) = ((d.extent.0 + dx).max(1.0), (d.extent.1 + dy).max(1.0));
            if let Err(e) = call(lua, d.id, "SetExtent", (w, h)) {
                editor.status(format!("SetExtent: {e}"));
            }
        } else if !d.anchors.is_empty() {
            let moved: Vec<_> = d.anchors.iter().map(|a| aa_ui::tree::Anchor { x: a.x + dx, y: a.y + dy, ..*a }).collect();
            if let Err(e) = set_anchors(lua, d.id, &moved) {
                editor.status(format!("anchors: {e}"));
            }
        }
    }
    let stroke = |c: egui::Color32, w: f32| egui::Stroke::new(w, c);
    if let Some(h) = st.hover.filter(|h| Some(*h) != st.selected) {
        let r = to_points(&rect_of(h), scale, ppp);
        painter.rect_stroke(r, 0.0, stroke(egui::Color32::from_rgb(90, 170, 255), 1.0), egui::StrokeKind::Outside);
        painter.text(r.left_top() - egui::vec2(0.0, 2.0), egui::Align2::LEFT_BOTTOM, label_of(lua, h), egui::FontId::proportional(12.0), egui::Color32::from_rgb(150, 200, 255));
    }
    if let Some(sel) = st.selected {
        let r = to_points(&rect_of(sel), scale, ppp);
        painter.rect_stroke(r, 0.0, stroke(egui::Color32::from_rgb(255, 200, 40), 2.0), egui::StrokeKind::Outside);
        // anchor targets
        let anchors = aa_ui::lua::ui(lua).tree.get(sel).anchors.clone();
        for a in anchors {
            let t = to_points(&rect_of(a.target), scale, ppp);
            let tp = t.min + egui::vec2(t.width() * a.rel.fx(), t.height() * a.rel.fy());
            let sp = r.min + egui::vec2(r.width() * a.point.fx(), r.height() * a.point.fy());
            painter.line_segment([tp, sp], stroke(egui::Color32::from_rgb(255, 120, 60), 1.0));
            painter.circle_filled(tp, 3.0, egui::Color32::from_rgb(255, 120, 60));
        }
        if editor.pick {
            painter.rect_filled(egui::Rect::from_min_size(r.max - egui::vec2(6.0, 6.0), egui::vec2(6.0, 6.0)), 0.0, egui::Color32::from_rgb(255, 200, 40));
        }
        painter.text(
            r.left_bottom() + egui::vec2(0.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("{}  {:.0},{:.0}  {:.0}x{:.0}", label_of(lua, sel), r.min.x, r.min.y, r.width(), r.height()),
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgb(255, 220, 120),
        );
    }
}

/// Replace an object's anchors (`RemoveAllAnchors` + `AddAnchor` each).
fn set_anchors(lua: &mlua::Lua, id: Id, anchors: &[aa_ui::tree::Anchor]) -> mlua::Result<()> {
    call(lua, id, "RemoveAllAnchors", ())?;
    for a in anchors {
        let target = aa_ui::lua::object_table(lua, a.target)?.map(mlua::Value::Table).unwrap_or(mlua::Value::Nil);
        call(lua, id, "AddAnchor", (a.point.name(), target, a.rel.name(), a.x, a.y))?;
    }
    Ok(())
}

fn tree_window(ctx: &egui::Context, editor: &mut Editor, st: &mut Inspect, lua: &mlua::Lua) {
    let mut open = editor.windows.tree;
    egui::Window::new("Widgets").open(&mut open).default_pos([4.0, 40.0]).default_size([300.0, 560.0]).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Find");
            ui.text_edit_singleline(&mut st.filter);
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut st.show_hidden, "hidden");
            ui.checkbox(&mut st.show_drawables, "drawables");
            if let Some(sel) = st.selected
                && ui.small_button("reveal selection").clicked()
            {
                st.select(Some(sel));
                st.reveal = true;
            }
        });
        ui.separator();
        // ancestors of the selection, to open them
        let reveal: HashSet<Id> = if std::mem::take(&mut st.reveal) {
            let g = aa_ui::lua::ui(lua);
            let mut set = HashSet::new();
            let mut cur = st.selected.and_then(|s| g.tree.get(s).parent);
            while let Some(p) = cur {
                set.insert(p);
                cur = g.tree.get(p).parent;
            }
            set
        } else {
            HashSet::new()
        };
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            if !st.filter.trim().is_empty() {
                let needle = st.filter.to_lowercase();
                let hits: Vec<(Id, String)> = {
                    let g = aa_ui::lua::ui(lua);
                    (0..g.tree.objects.len() as Id)
                        .filter(|&id| {
                            let o = g.tree.get(id);
                            o.alive && (st.show_drawables || o.is_widget()) && (st.show_hidden || g.tree.visible(id))
                        })
                        .filter_map(|id| {
                            let o = g.tree.get(id);
                            let class = match &o.kind {
                                Kind::Widget(w) => w.class.as_str(),
                                Kind::Drawable(d) => d.class,
                            };
                            let text = o.text().map(|t| t.text.as_str()).unwrap_or("");
                            let hay = format!("{} {} {}", o.name, class, text).to_lowercase();
                            hay.contains(&needle).then(|| (id, format!("{} <{class}> {}", if o.name.is_empty() { format!("#{id}") } else { o.name.clone() }, text.chars().take(30).collect::<String>())))
                        })
                        .take(500)
                        .collect()
                };
                ui.label(format!("{} found (first 500)", hits.len()));
                for (id, label) in hits {
                    let r = ui.selectable_label(st.selected == Some(id), label);
                    if r.clicked() {
                        st.select(Some(id));
                    }
                    if r.hovered() {
                        st.hover = Some(id);
                    }
                }
                return;
            }
            let tops: Vec<Id> = aa_ui::lua::ui(lua).tree.top.clone();
            for id in tops {
                node(ui, st, lua, id, &reveal, 0);
            }
        });
    });
    editor.windows.tree = open;
}

fn node(ui: &mut egui::Ui, st: &mut Inspect, lua: &mlua::Lua, id: Id, reveal: &HashSet<Id>, depth: usize) {
    let (visible, label, children) = {
        let g = aa_ui::lua::ui(lua);
        let o = g.tree.get(id);
        if !o.alive || depth > 40 {
            return;
        }
        let visible = g.tree.visible(id);
        let mut children: Vec<Id> = Vec::new();
        if let Kind::Widget(w) = &o.kind {
            children.extend(w.children.iter().copied().filter(|&c| st.show_hidden || g.tree.visible(c)));
            if st.show_drawables {
                children.extend(w.drawables.iter().copied());
            }
        }
        drop(g);
        (visible, label_of(lua, id), children)
    };
    if !visible && !st.show_hidden {
        return;
    }
    let text = if visible { egui::RichText::new(label) } else { egui::RichText::new(label).weak() };
    let selected = st.selected == Some(id);
    let mut clicked = false;
    let mut hovered = false;
    if children.is_empty() {
        let r = ui.selectable_label(selected, text);
        clicked = r.clicked();
        hovered = r.hovered();
        if selected && !reveal.is_empty() {
            r.scroll_to_me(Some(egui::Align::Center));
        }
    } else {
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(("node", id)), false);
        if reveal.contains(&id) {
            state.set_open(true);
        }
        state
            .show_header(ui, |ui| {
                let r = ui.selectable_label(selected, text);
                clicked = r.clicked();
                hovered = r.hovered();
                if selected && !reveal.is_empty() {
                    r.scroll_to_me(Some(egui::Align::Center));
                }
            })
            .body(|ui| {
                for c in children {
                    node(ui, st, lua, c, reveal, depth + 1);
                }
            });
    }
    if clicked {
        st.select(Some(id));
    }
    if hovered {
        st.hover = Some(id);
    }
}

fn inspector(ctx: &egui::Context, editor: &mut Editor, st: &mut Inspect, code: &mut Code, lua: &mlua::Lua) {
    let mut open = editor.windows.inspector;
    egui::Window::new("Inspector").open(&mut open).default_pos([1500.0, 40.0]).default_size([410.0, 640.0]).max_width(460.0).show(ctx, |ui| {
        let Some(id) = st.selected else {
            ui.label("Nothing selected. Pick a widget on screen (F2, or Alt+click) or in the widget tree.");
            return;
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            inspector_body(ui, editor, st, code, lua, id);
        });
    });
    editor.windows.inspector = open;
}

struct Snapshot {
    name: String,
    class: String,
    widget: bool,
    parent: Option<Id>,
    rect: Rect,
    width: f32,
    height: f32,
    shown: bool,
    visible: bool,
    alpha: f32,
    anchors: Vec<aa_ui::tree::Anchor>,
    layer: String,
    text: Option<(String, String)>,
    drawables: Vec<Id>,
    children: usize,
    handlers: Vec<String>,
    texture: Option<String>,
    color: [f32; 4],
    coords: Option<[f32; 4]>,
}

fn snapshot(lua: &mlua::Lua, id: Id) -> Snapshot {
    let g = aa_ui::lua::ui(lua);
    let o = g.tree.get(id);
    let (class, widget, layer, drawables, children, texture, color, coords) = match &o.kind {
        Kind::Widget(w) => (w.class.clone(), true, format!("{:?}", w.layer), w.drawables.clone(), w.children.len(), None, [1.0; 4], None),
        Kind::Drawable(d) => (d.class.to_string(), false, format!("{:?}", d.layer), Vec::new(), 0, d.texture.clone(), d.color, d.coords),
    };
    let text = o.text().map(|t| (t.text.clone(), format!("{} {:.0}px {:?} rgba({:.2}, {:.2}, {:.2}, {:.2})", t.style.font, t.style.size, t.style.align, t.style.color[0], t.style.color[1], t.style.color[2], t.style.color[3])));
    Snapshot {
        name: o.name.clone(),
        class,
        widget,
        parent: o.parent,
        rect: g.tree.rect(id),
        width: o.width,
        height: o.height,
        shown: o.shown,
        visible: g.tree.visible(id),
        alpha: o.alpha,
        anchors: o.anchors.clone(),
        layer,
        text,
        drawables,
        children,
        handlers: g.handlers.get(&id).cloned().unwrap_or_default(),
        texture,
        color,
        coords,
    }
}

fn site_button(ui: &mut egui::Ui, editor: &mut Editor, site: &track::Site) {
    // the folder and file name; the whole path on hover
    let path = site.path.trim_start_matches("scriptsbin/").trim_end_matches(".alb");
    let short = path.rsplitn(3, '/').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("/");
    if ui.link(format!("{short}:{}", site.line)).on_hover_text(format!("{path}:{}
Open in the script editor", site.line)).clicked() {
        editor.open_script = Some((site.path.clone(), Some(site.line)));
    }
}

fn inspector_body(ui: &mut egui::Ui, editor: &mut Editor, st: &mut Inspect, code: &mut Code, lua: &mlua::Lua, id: Id) {
    let s = snapshot(lua, id);
    ui.heading(if s.name.is_empty() { format!("#{id}") } else { s.name.clone() });
    ui.label(format!("{} {} · id {id} · layer {}", if s.widget { "widget" } else { "drawable" }, s.class, s.layer));
    // ancestors
    ui.horizontal_wrapped(|ui| {
        ui.label("in");
        let mut chain = Vec::new();
        let mut cur = s.parent;
        while let Some(p) = cur {
            chain.push(p);
            cur = aa_ui::lua::ui(lua).tree.get(p).parent;
        }
        for p in chain.into_iter().rev() {
            if ui.link(label_of(lua, p)).clicked() {
                st.select(Some(p));
            }
            ui.label("›");
        }
    });
    // script paths (computed on demand: the walk visits every table)
    match &st.paths {
        Some((pid, paths)) if *pid == id => {
            for p in paths {
                ui.horizontal(|ui| {
                    ui.monospace(p);
                    if ui.small_button("copy").clicked() {
                        ui.ctx().copy_text(p.clone());
                    }
                });
            }
            if paths.is_empty() {
                ui.weak("not reachable from a global or an upvalue");
            }
        }
        _ => {
            if ui.small_button("Find Lua paths").on_hover_text("How scripts reach this object (`window.titleBar.closeButton`), from _G and upvalues").clicked() {
                st.paths = Some((id, lua_paths(lua, id)));
            }
        }
    }
    ui.separator();
    ui.label(format!(
        "rect {:.0}, {:.0}  {:.0} x {:.0}   {}",
        s.rect.x,
        s.rect.y,
        s.rect.w,
        s.rect.h,
        if s.visible { "visible" } else if s.shown { "shown, parent hidden" } else { "hidden" }
    ));
    // show / alpha
    ui.horizontal(|ui| {
        let mut shown = s.shown;
        if ui.checkbox(&mut shown, "Show").changed() {
            match call(lua, id, "Show", shown) {
                Ok(()) => st.record(id, Edit::Show(shown)),
                Err(e) => editor.status(format!("Show: {e}")),
            }
        }
        let mut alpha = s.alpha;
        if ui.add(egui::Slider::new(&mut alpha, 0.0..=1.0).text("alpha")).changed() {
            match call(lua, id, "SetAlpha", alpha) {
                Ok(()) => st.record(id, Edit::Alpha(alpha)),
                Err(e) => editor.status(format!("SetAlpha: {e}")),
            }
        }
    });
    // extent
    ui.horizontal(|ui| {
        ui.label("Extent");
        let (mut w, mut h) = (s.width, s.height);
        let cw = ui.add(egui::DragValue::new(&mut w).speed(1.0).range(0.0..=8192.0)).changed();
        let ch = ui.add(egui::DragValue::new(&mut h).speed(1.0).range(0.0..=8192.0)).changed();
        if cw || ch {
            match call(lua, id, "SetExtent", (w, h)) {
                Ok(()) => st.record(id, Edit::Extent(w, h)),
                Err(e) => editor.status(format!("SetExtent: {e}")),
            }
        }
    });
    // anchors
    ui.collapsing(format!("Anchors ({})", s.anchors.len()), |ui| {
        let mut anchors = s.anchors.clone();
        let mut changed = None;
        let mut remove = None;
        egui::Grid::new("anchors").striped(true).show(ui, |ui| {
            for (i, a) in anchors.iter_mut().enumerate() {
                point_combo(ui, ("ap", i), &mut a.point, &mut changed, i);
                if ui.link(label_of(lua, a.target)).on_hover_text("select the anchor target").clicked() {
                    st.select(Some(a.target));
                }
                point_combo(ui, ("ar", i), &mut a.rel, &mut changed, i);
                if ui.add(egui::DragValue::new(&mut a.x).speed(1.0)).changed() {
                    changed = Some(i);
                }
                if ui.add(egui::DragValue::new(&mut a.y).speed(1.0)).changed() {
                    changed = Some(i);
                }
                if ui.small_button("x").on_hover_text("remove this anchor").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            anchors.remove(i);
            changed = Some(usize::MAX);
        }
        ui.horizontal(|ui| {
            ui.label("add anchor to");
            ui.add(egui::TextEdit::singleline(&mut st.target_buf).hint_text("widget name / parent").desired_width(120.0));
            if ui.button("+").clicked() {
                let target = if st.target_buf.trim().is_empty() || st.target_buf.trim() == "parent" {
                    s.parent
                } else {
                    aa_ui::lua::ui(lua).tree.find(st.target_buf.trim())
                };
                match target {
                    Some(t) => {
                        anchors.push(aa_ui::tree::Anchor { point: Point::TopLeft, target: t, rel: Point::TopLeft, x: 0.0, y: 0.0 });
                        changed = Some(anchors.len() - 1);
                    }
                    None => editor.status(format!("no widget named {:?}", st.target_buf)),
                }
            }
        });
        if let Some(i) = changed {
            match set_anchors(lua, id, &anchors) {
                Ok(()) => {
                    for (j, a) in anchors.iter().enumerate() {
                        if i == usize::MAX || i == j {
                            st.record(id, Edit::Anchor { point: a.point.name().into(), rel: a.rel.name().into(), x: a.x, y: a.y });
                        }
                    }
                }
                Err(e) => editor.status(format!("anchors: {e}")),
            }
        }
    });
    // text
    if let Some((text, style)) = &s.text {
        ui.collapsing("Text", |ui| {
            ui.weak(style);
            let buf = match &mut st.text_buf {
                Some((tid, b)) if *tid == id => b,
                _ => {
                    st.text_buf = Some((id, text.clone()));
                    &mut st.text_buf.as_mut().unwrap().1
                }
            };
            ui.add(egui::TextEdit::multiline(buf).desired_rows(2).desired_width(f32::INFINITY));
            if ui.button("Set text").clicked() {
                let t = buf.clone();
                match call(lua, id, "SetText", t.clone()) {
                    Ok(()) => st.record(id, Edit::Text(t)),
                    Err(e) => editor.status(format!("SetText: {e}")),
                }
            }
        });
    }
    // drawable look
    if !s.widget {
        ui.collapsing("Look", |ui| {
            let buf = match &mut st.texture_buf {
                Some((tid, b)) if *tid == id => b,
                _ => {
                    st.texture_buf = Some((id, s.texture.clone().unwrap_or_default()));
                    &mut st.texture_buf.as_mut().unwrap().1
                }
            };
            ui.horizontal(|ui| {
                ui.label("texture");
                ui.text_edit_singleline(buf);
            });
            if ui.button("Set texture").clicked() {
                let t = buf.clone();
                match call(lua, id, "SetTexture", t.clone()) {
                    Ok(()) => st.record(id, Edit::Texture(t)),
                    Err(e) => editor.status(format!("SetTexture: {e}")),
                }
            }
            let mut c = s.color;
            ui.horizontal(|ui| {
                ui.label("colour");
                if ui.color_edit_button_rgba_unmultiplied(&mut c).changed() {
                    match call(lua, id, "SetColor", (c[0], c[1], c[2], c[3])) {
                        Ok(()) => st.record(id, Edit::Color(c)),
                        Err(e) => editor.status(format!("SetColor: {e}")),
                    }
                }
            });
            if let Some(mut co) = s.coords {
                ui.horizontal(|ui| {
                    ui.label("coords");
                    let mut ch = false;
                    for v in co.iter_mut() {
                        ch |= ui.add(egui::DragValue::new(v).speed(1.0)).changed();
                    }
                    if ch {
                        match call(lua, id, "SetCoords", (co[0], co[1], co[2], co[3])) {
                            Ok(()) => st.record(id, Edit::Coords(co)),
                            Err(e) => editor.status(format!("SetCoords: {e}")),
                        }
                    }
                });
            }
        });
    }
    if !s.drawables.is_empty() {
        ui.collapsing(format!("Drawables ({})", s.drawables.len()), |ui| {
            for d in &s.drawables {
                let (label, tex) = {
                    let g = aa_ui::lua::ui(lua);
                    let o = g.tree.get(*d);
                    let tex = o.drawable().and_then(|x| x.texture.clone()).unwrap_or_default();
                    (o.shown, tex)
                };
                if ui.selectable_label(false, format!("{}{}  {}", if label { "" } else { "(hidden) " }, label_of(lua, *d), tex)).clicked() {
                    st.select(Some(*d));
                }
            }
        });
    }
    if s.children > 0 {
        ui.label(format!("{} children", s.children));
    }
    if !s.handlers.is_empty() {
        ui.collapsing(format!("Handlers ({})", s.handlers.len()), |ui| {
            ui.label(s.handlers.join(", "));
        });
    }
    // script sites
    ui.separator();
    ui.strong("Script");
    match track::created(lua, id) {
        Some(c) => {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("created by {} at", c.method));
                for site in &c.stack {
                    site_button(ui, editor, site);
                }
            });
        }
        None => {
            ui.weak("creation not recorded (made by the toolkit, or before the scripts ran)");
        }
    }
    let sites = track::sites(lua, id);
    if !sites.is_empty() {
        for (key, c) in &sites {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{key} ({})", c.method));
                for site in c.stack.iter().take(2) {
                    site_button(ui, editor, site);
                }
            });
        }
    }
    // pending changes
    let edits: Vec<Edit> = st.edits.get(&id).map(|m| m.values().cloned().collect()).unwrap_or_default();
    ui.separator();
    ui.strong(format!("Changes ({})", edits.len()));
    for e in &edits {
        ui.monospace(format!("{}  {e:?}", e.method()));
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!edits.is_empty(), egui::Button::new("Write into script…"))
            .on_hover_text("Rewrite the script lines that set these properties (or add lines after them); shows a preview first")
            .clicked()
        {
            st.preview = Some(plan(lua, code, editor, st, id));
        }
        if ui.add_enabled(!edits.is_empty(), egui::Button::new("Copy as Lua")).clicked() {
            let recv = st.paths.as_ref().filter(|(p, _)| *p == id).and_then(|(_, p)| p.first().cloned()).unwrap_or_else(|| "w".into());
            let lua_text: Vec<String> = edits.iter().map(|e| snippet(&recv, e)).collect();
            ui.ctx().copy_text(lua_text.join("\n"));
            editor.status("copied the changes as Lua");
        }
        if ui.add_enabled(!edits.is_empty(), egui::Button::new("Forget")).clicked() {
            st.edits.remove(&id);
        }
    });
    if !st.stack.is_empty() {
        ui.collapsing("Under the pointer", |ui| {
            for &w in &st.stack.clone() {
                if ui.selectable_label(st.selected == Some(w), label_of(lua, w)).clicked() {
                    st.select(Some(w));
                }
            }
        });
    }
}

fn snippet(recv: &str, e: &Edit) -> String {
    match e {
        Edit::Anchor { point, rel, x, y } => format!("{recv}:AddAnchor(\"{point}\", <target>, \"{rel}\", {}, {})", patch::num(*x), patch::num(*y)),
        Edit::Extent(w, h) => format!("{recv}:SetExtent({}, {})", patch::num(*w), patch::num(*h)),
        Edit::Show(b) => format!("{recv}:Show({b})"),
        Edit::Text(t) => format!("{recv}:SetText({t:?})"),
        Edit::Alpha(a) => format!("{recv}:SetAlpha({})", patch::num(*a)),
        Edit::Color(c) => format!("{recv}:SetColor({}, {}, {}, {})", patch::num(c[0]), patch::num(c[1]), patch::num(c[2]), patch::num(c[3])),
        Edit::Texture(t) => format!("{recv}:SetTexture({t:?})"),
        Edit::Coords(c) => format!("{recv}:SetCoords({}, {}, {}, {})", patch::num(c[0]), patch::num(c[1]), patch::num(c[2]), patch::num(c[3])),
    }
}

fn point_combo(ui: &mut egui::Ui, salt: (&str, usize), p: &mut Point, changed: &mut Option<usize>, i: usize) {
    egui::ComboBox::from_id_salt(salt).width(100.0).selected_text(p.name()).show_ui(ui, |ui| {
        for q in POINTS {
            if ui.selectable_value(p, q, q.name()).changed() {
                *changed = Some(i);
            }
        }
    });
}

/// Paths of one object from `_G` and upvalues (the dump's walk, 8 steps deep).
fn lua_paths(lua: &mlua::Lua, id: Id) -> Vec<String> {
    const WALK: &str = r#"
return function(target, maxDepth)
  local out = {}
  local SKIP = { _G = true, package = true, string = true, table = true, math = true, os = true,
                 io = true, debug = true, coroutine = true, AAED = true, FAKE = true }
  local seen, queue, qi = {}, {}, 1
  local function visit(v, kp, depth)
    if v == target and #out < 6 then out[#out + 1] = kp end
    if seen[v] then return end
    seen[v] = true
    queue[#queue + 1] = { v, kp, depth }
  end
  for k, v in next, _G do
    local tv = type(v)
    if type(k) == "string" and not SKIP[k] and (tv == "table" or tv == "function") then visit(v, k, 1) end
  end
  while qi <= #queue and #out < 6 do
    local item = queue[qi]; qi = qi + 1
    local t, path, depth = item[1], item[2], item[3]
    if depth < maxDepth then
      if type(t) == "function" then
        local info = debug.getinfo(t, "S")
        if info and info.what ~= "C" then
          local i = 1
          while true do
            local n, v = debug.getupvalue(t, i)
            if not n then break end
            if type(v) == "table" and v ~= _G then visit(v, path .. "^" .. n, depth + 1) end
            i = i + 1
          end
        end
      else
        for k, v in next, t do
          local tv = type(v)
          if (tv == "table" or tv == "function") and k ~= "__index" then
            local kp
            if type(k) == "number" then kp = string.format("%s[%d]", path, k)
            elseif type(k) == "string" then kp = path .. "." .. k end
            if kp then visit(v, kp, depth + 1) end
          end
        end
      end
    end
  end
  table.sort(out, function(a, b) return #a < #b end)
  return out
end
"#;
    let run = || -> mlua::Result<Vec<String>> {
        let Some(t) = aa_ui::lua::object_table(lua, id)? else { return Ok(Vec::new()) };
        let f: mlua::Function = lua.load(WALK).set_name("@ui-editor/paths").eval()?;
        let r: mlua::Table = f.call((t, 8))?;
        Ok(r.sequence_values::<String>().filter_map(Result::ok).collect())
    };
    run().unwrap_or_else(|e| vec![format!("(walk failed: {e})")])
}

/// Patches for every change of object `id`, in descending line order per file so earlier ones stay valid.
fn plan(lua: &mlua::Lua, code: &mut Code, editor: &Editor, st: &Inspect, id: Id) -> Vec<Planned> {
    let edits: Vec<Edit> = st.edits.get(&id).map(|m| m.values().cloned().collect()).unwrap_or_default();
    let sites = track::sites(lua, id);
    let created = track::created(lua, id);
    let origin = |path: &str| screens::LAST.with(|l| l.borrow().log.ran.iter().rev().find(|(p, _)| p == path).map(|(_, o)| *o));
    let mut out = Vec::new();
    let mut by_file: BTreeMap<String, Vec<(usize, Edit, Option<String>)>> = BTreeMap::new();
    for e in edits {
        let key = e.key();
        let site = sites.iter().find(|(k, _)| *k == key).or_else(|| match &e {
            Edit::Extent(..) => sites.iter().find(|(k, _)| k == "width" || k == "height"),
            _ => None,
        });
        let (path, line, method) = match (site, &created) {
            (Some((_, c)), _) => match c.stack.first() {
                Some(s) => (s.path.clone(), s.line, Some(c.method.clone())),
                None => continue,
            },
            (None, Some(c)) => match c.stack.first() {
                Some(s) => (s.path.clone(), s.line, None),
                None => continue,
            },
            (None, None) => {
                out.push(Planned { path: String::new(), result: Err(format!("{}: no script line recorded for this object", e.method())), id, key });
                continue;
            }
        };
        by_file.entry(path).or_default().push((line, e, method));
    }
    let created_site = created.as_ref().and_then(|c| c.stack.first().map(|s| (s.path.clone(), s.line, c.method.clone())));
    for (path, mut list) in by_file {
        if origin(&path) == Some(crate::loader::Origin::Alb) {
            for (_, e, _) in list {
                out.push(Planned { path: path.clone(), result: Err("this script ran from its .alb, so its line numbers do not match the source: tick \"run decompiled sources\" and rebuild".into()), id, key: e.key() });
            }
            continue;
        }
        let mut text = match code.source_text(editor, &path) {
            Ok(t) => t,
            Err(e) => {
                out.push(Planned { path: path.clone(), result: Err(e), id, key: String::new() });
                continue;
            }
        };
        list.sort_by_key(|x| std::cmp::Reverse(x.0));
        for (line, e, method) in list {
            let created_here = created_site.as_ref().filter(|c| c.0 == path).map(|c| (c.1, c.2.as_str()));
            let r = match &method {
                Some(m) => patch::apply(&text, Some((line, m.as_str())), None, &e),
                None => patch::apply(&text, None, created_here, &e),
            };
            if let Ok(p) = &r {
                text = p.text.clone();
            }
            out.push(Planned { path: path.clone(), result: r, id, key: e.key() });
        }
    }
    out
}

fn preview_window(ctx: &egui::Context, editor: &mut Editor, st: &mut Inspect, code: &mut Code) {
    let Some(planned) = st.preview.take() else { return };
    let mut keep = true;
    let mut apply = None;
    egui::Window::new("Write into script").collapsible(false).default_pos([600.0, 200.0]).show(ctx, |ui| {
        for p in &planned {
            match &p.result {
                Ok(r) => ui.label(format!("{}  {}", p.path.trim_start_matches("scriptsbin/"), r.what)),
                Err(e) => ui.colored_label(egui::Color32::from_rgb(255, 120, 100), format!("{} {}: {e}", p.path.trim_start_matches("scriptsbin/"), p.key)),
            };
        }
        ui.label("A line inside a loop or a helper applies to every object that runs it.");
        ui.horizontal(|ui| {
            let ok = planned.iter().any(|p| p.result.is_ok());
            if ui.add_enabled(ok, egui::Button::new("Apply (open in editor)")).clicked() {
                apply = Some(false);
            }
            if ui.add_enabled(ok, egui::Button::new("Apply, save and rebuild")).clicked() {
                apply = Some(true);
            }
            if ui.button("Cancel").clicked() {
                keep = false;
            }
        });
    });
    if let Some(save) = apply {
        st.preview = Some(planned);
        preview_apply(editor, st, code, save);
        return;
    }
    if keep {
        st.preview = Some(planned);
    }
}

/// Apply the planned patches to the editor tabs (and save them: compile + rebuild).
fn preview_apply(editor: &mut Editor, st: &mut Inspect, code: &mut Code, save: bool) {
    let Some(planned) = st.preview.take() else { return };
    {
        // the last successful patch per file holds the file's final text
        let mut finals: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for p in &planned {
            if let Ok(r) = &p.result {
                finals.insert(p.path.clone(), (r.text.clone(), r.line));
                if let Some(m) = st.edits.get_mut(&p.id) {
                    m.remove(&p.key);
                }
            }
        }
        for (path, (text, line)) in finals {
            match code.set_text(editor, &path, text, Some(line)) {
                Ok(i) if save => code.save(editor, i),
                Ok(_) => {}
                Err(e) => editor.status(format!("{path}: {e}")),
            }
        }
    }
}
