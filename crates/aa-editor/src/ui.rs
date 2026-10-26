//! The panels: menu bar, browser (NPCs / doodads / items / quests of the game database), inspector (the selected
//! spawn, the terrain brush, view settings), the database row editor and the status bar.

use aa_world::{WorldClock, bevy_to_cry, cry_to_bevy};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::camera::{EditorCamera, FlySpeed};
use crate::db::{Field, GameData, Table, component_kind};
use crate::spawns::{Kind, SpawnStore};
use crate::terrain_edit::{Brush, BrushKind, Heights};
use crate::model_preview::{ModelPreview, Subject};
use crate::tools::Cursor;
use crate::{Editor, Mode, Request};

/// The row editor's open row.
#[derive(Clone, Debug)]
pub struct RowEdit {
    pub table: Table,
    pub id: u32,
    pub fields: Vec<Field>,
    pub name: String,
    pub description: Option<String>,
    original_name: String,
    original_description: Option<String>,
    confirm_delete: bool,
    filter: String,
}

impl RowEdit {
    pub fn open(game: &GameData, table: Table, id: u32) -> Result<RowEdit, String> {
        let fields = game.row(table, id).map_err(|e| e.to_string())?;
        let name = game.localized(table, "name", id).unwrap_or_default();
        let description = matches!(table, Table::Items).then(|| game.localized(table, "description", id).unwrap_or_default());
        Ok(RowEdit {
            table,
            id,
            fields,
            original_name: name.clone(),
            name,
            original_description: description.clone(),
            description,
            confirm_delete: false,
            filter: String::new(),
        })
    }

    fn changed(&self) -> bool {
        self.name != self.original_name || self.description != self.original_description || self.fields.iter().any(|f| f.text != f.original)
    }
}

/// Screen rectangles (logical pixels) of the panels drawn last frame, plus whether egui itself wants the pointer
/// (a window, a menu, a drag): the world tools ignore the mouse there.
#[derive(Resource, Default)]
pub struct UiArea {
    rects: Vec<egui::Rect>,
    egui_wants_pointer: bool,
    /// The 3D view left between the panels (logical points).
    pub view: Option<egui::Rect>,
}

impl UiArea {
    /// Another panel rectangle (logical points) the world tools must ignore.
    pub fn add(&mut self, r: egui::Rect, pixels_per_point: f32) {
        self.rects.push(egui::Rect::from_min_max((r.min.to_vec2() * pixels_per_point).to_pos2(), (r.max.to_vec2() * pixels_per_point).to_pos2()));
    }

    pub fn contains(&self, logical: Vec2) -> bool {
        self.egui_wants_pointer || self.rects.iter().any(|r| r.contains(egui::pos2(logical.x, logical.y)))
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<UiArea>().add_systems(EguiPrimaryContextPass, panels).add_systems(Update, save_spawns);
}

fn save_spawns(mut editor: ResMut<Editor>, mut store: ResMut<SpawnStore>, game: Res<GameData>) {
    let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::SaveSpawns)) else { return };
    editor.requests.remove(i);
    let changed = store.changed_count();
    if changed == 0 {
        editor.status("spawns: nothing to save");
        return;
    }
    let mirrors = editor.paths.mirrors.clone();
    let result = store.save(&mirrors, |kind, id| {
        game.name(
            match kind {
                Kind::Npc => Table::Npcs,
                Kind::Doodad => Table::Doodads,
            },
            id,
        )
    });
    match result {
        Ok(files) => {
            let names: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
            editor.status(format!("spawns: {changed} change(s) saved to {} (restart the server to load them)", names.join(", ")));
        }
        Err(e) => editor.status(format!("spawn save failed: {e:#}")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn panels(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut store: ResMut<SpawnStore>,
    mut game: ResMut<GameData>,
    mut brush: ResMut<Brush>,
    heights: Res<Heights>,
    cursor: Res<Cursor>,
    mut clock: ResMut<WorldClock>,
    mut speed: ResMut<FlySpeed>,
    diagnostics: Res<DiagnosticsStore>,
    cam: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    mut area: ResMut<UiArea>,
    mut preview: ResMut<ModelPreview>,
    worlds: Res<crate::worlds::Worlds>,
    (mut paths, housing): (Option<ResMut<crate::paths::AiPaths>>, Option<Res<crate::housing::Housing>>),
    mut fonts: Local<bool>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !*fonts {
        *fonts = true;
        install_fonts(ctx);
    }
    area.rects.clear();
    area.egui_wants_pointer = ctx.egui_wants_pointer_input() || ctx.is_pointer_over_egui() || ctx.egui_is_using_pointer();
    let fps = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()).unwrap_or(0.0);

    if editor.playing {
        egui::Area::new("play".into()).anchor(egui::Align2::CENTER_TOP, [0.0, 8.0]).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("■ Stop (Esc)").clicked() {
                        editor.requests.push(Request::Stop);
                    }
                    ui.label(format!("Playing — WASD, Shift run, Space jump, right mouse turn · {fps:.0} fps"));
                });
            });
        });
        return Ok(());
    }

    // the panels share a root ui covering the window (egui 0.36 panels are laid out inside a parent ui)
    let mut root = egui::Ui::new(
        ctx.clone(),
        "editor panels".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );
    let ppp = ctx.pixels_per_point();
    let mut push = |r: egui::Rect| area.rects.push(egui::Rect::from_min_max((r.min.to_vec2() * ppp).to_pos2(), (r.max.to_vec2() * ppp).to_pos2()));
    let unsaved_paths = paths.as_ref().map_or(0, |p| p.changed_count()) + housing.as_ref().map_or(0, |h| h.changed_count());
    push(menu_bar(&mut root, &mut editor, &mut store, &heights, &mut preview, &worlds, unsaved_paths, fps));
    push(status_bar(&mut root, &editor, &store, &heights, &cursor, &cam));
    push(browser(&mut root, &mut editor, &game, &mut preview));
    push(inspector(&mut root, &mut editor, &mut store, &game, &mut brush, &mut clock, &mut speed, paths.as_deref_mut()));
    area.view = Some(root.available_rect_before_wrap());
    row_editor(ctx, &mut editor, &mut game, &store, &mut preview);
    labels(ctx, &editor, &store, &game, &cursor, &cam);
    Ok(())
}

/// Korean names (untranslated rows) need a Hangul font: Windows' Malgun Gothic as a fallback, when present.
fn install_fonts(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read("C:/Windows/Fonts/malgun.ttf") else { return };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("hangul".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("hangul".into());
    }
    ctx.set_fonts(fonts);
}

#[allow(clippy::too_many_arguments)]
fn menu_bar(root: &mut egui::Ui, editor: &mut Editor, store: &mut SpawnStore, heights: &Heights, preview: &mut ModelPreview, worlds: &crate::worlds::Worlds, unsaved_paths: usize, fps: f64) -> egui::Rect {
    egui::Panel::top("menu").show(root, |ui| {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Save all  (Ctrl+S)").clicked() {
                    editor.requests.extend([Request::SaveSpawns, Request::SaveTerrain, Request::SaveObjects, Request::SavePaths, Request::SaveHousing]);
                }
                if ui.button("Save spawns → server data").clicked() {
                    editor.requests.push(Request::SaveSpawns);
                }
                if ui.button("Save terrain → pak overlay").clicked() {
                    editor.requests.push(Request::SaveTerrain);
                }
                if ui.button("Save AI paths → server data").clicked() {
                    editor.requests.push(Request::SavePaths);
                }
                if ui.button("Save housing zones → overlay, server, database").clicked() {
                    editor.requests.push(Request::SaveHousing);
                }
                ui.separator();
                ui.label(format!("Spawns: {}", editor.paths.spawn_dir.display()));
                for m in &editor.paths.mirrors {
                    ui.label(format!("  + copy to {}", m.display()));
                }
                ui.label(format!("Database: {}", editor.paths.db.display()));
                ui.label(format!("Pak overlay: {}", editor.paths.overlay.display()));
            });
            ui.menu_button("Edit", |ui| {
                if ui.add_enabled(store.can_undo(), egui::Button::new("Undo spawn edit  (Ctrl+Z)")).clicked() {
                    store.undo();
                }
                if ui.add_enabled(store.can_redo(), egui::Button::new("Redo spawn edit  (Ctrl+Y)")).clicked() {
                    store.redo();
                }
                if ui.add_enabled(heights.can_undo(), egui::Button::new("Undo terrain stroke")).clicked() {
                    editor.requests.push(Request::UndoTerrain);
                }
            });
            crate::worlds::menu(ui, editor, worlds);
            if ui.button("🗺 Teleport (M)").on_hover_text("A map to click on, zones, portals, bookmarks, NPCs by name, coordinates").clicked() {
                editor.requests.push(Request::OpenTeleport);
            }
            ui.separator();
            for (mode, label) in [(Mode::Select, "Select (1)"), (Mode::Place, "Place (2)"), (Mode::Terrain, "Terrain (3)"), (Mode::Objects, "Objects (4)"), (Mode::Paths, "Paths (5)"), (Mode::Housing, "Housing (6)")] {
                if ui.selectable_label(editor.mode == mode, label).clicked() {
                    editor.mode = mode;
                }
            }
            ui.separator();
            if ui.button("▶ Play (F5)").clicked() {
                editor.requests.push(Request::Play);
            }
            if ui.button("Play on server").clicked() {
                editor.requests.push(Request::PlayOnServer);
            }
            if ui.selectable_label(preview.open, "Model preview").on_hover_text("A turntable view of the row under the pointer, the open row or the selected spawn").clicked() {
                preview.open = !preview.open;
            }
            ui.separator();
            let unsaved = store.changed_count() + heights.unsaved.len() + unsaved_paths;
            if unsaved > 0 {
                ui.colored_label(egui::Color32::from_rgb(255, 170, 60), format!("● {unsaved} unsaved"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{fps:.0} fps"));
            });
        });
    })
    .response
    .rect
}

fn status_bar(
    root: &mut egui::Ui,
    editor: &Editor,
    store: &SpawnStore,
    heights: &Heights,
    cursor: &Cursor,
    cam: &Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
) -> egui::Rect {
    egui::Panel::bottom("status").show(root, |ui| {
        ui.horizontal(|ui| {
            if let Ok((_, t)) = cam.single() {
                let p = bevy_to_cry(t.translation());
                ui.monospace(format!("camera {:.0}, {:.0}, {:.0}", p.x, p.y, p.z));
            }
            if let Some(g) = cursor.ground {
                ui.monospace(format!("· cursor {:.1}, {:.1}, {:.1}", g.x, g.y, g.z));
            }
            ui.separator();
            ui.label(format!("{} spawns · {} changed · {} terrain cells edited", store.spawns.len(), store.changed_count(), heights.edited.len()));
            ui.separator();
            ui.label(&editor.status);
        });
    })
    .response
    .rect
}

fn browser(root: &mut egui::Ui, editor: &mut Editor, game: &GameData, preview: &mut ModelPreview) -> egui::Rect {
    egui::Panel::left("browser").resizable(true).default_size(300.0).show(root, |ui| {
        ui.heading("Browser");
        ui.horizontal(|ui| {
            for t in Table::ALL {
                if ui.selectable_label(editor.browser == t, t.label()).clicked() {
                    editor.browser = t;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.text_edit_singleline(&mut editor.search);
        });
        let needle = editor.search.to_lowercase();
        let list = &game.lists[&editor.browser];
        let hits: Vec<usize> = list.iter().enumerate().filter(|(_, e)| e.matches(&needle)).map(|(i, _)| i).collect();
        ui.label(format!("{} of {}", hits.len(), list.len()));
        match editor.browser {
            Table::Npcs | Table::Doodads => {
                ui.small("Click to place (Shift-click in the world keeps placing) · edit opens the row");
            }
            _ => {
                ui.small("Click to edit the row");
            }
        }
        ui.separator();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_h, hits.len(), |ui, range| {
            for &i in &hits[range] {
                let e = &list[i];
                ui.horizontal(|ui| {
                    let placing = editor.place.is_some_and(|(k, id)| id == e.id && matches!((k, editor.browser), (Kind::Npc, Table::Npcs) | (Kind::Doodad, Table::Doodads)));
                    let open = editor.row.as_ref().is_some_and(|r| r.table == editor.browser && r.id == e.id);
                    let label = ui.selectable_label(placing || open, format!("{:>6}  {}", e.id, e.name));
                    if label.hovered() {
                        preview.hover = Some(Subject::of_row(editor.browser, e.id));
                    }
                    let label = if !e.model.is_empty() { label.on_hover_text(&e.model) } else { label };
                    if label.clicked() {
                        match editor.browser {
                            Table::Npcs => {
                                editor.place = Some((Kind::Npc, e.id));
                                editor.mode = Mode::Place;
                            }
                            Table::Doodads => {
                                editor.place = Some((Kind::Doodad, e.id));
                                editor.mode = Mode::Place;
                            }
                            t => match RowEdit::open(game, t, e.id) {
                                Ok(r) => editor.row = Some(r),
                                Err(err) => editor.status(err),
                            },
                        }
                    }
                    if matches!(editor.browser, Table::Npcs | Table::Doodads) && ui.small_button("edit").on_hover_text("Edit the database row").clicked() {
                        match RowEdit::open(game, editor.browser, e.id) {
                            Ok(r) => editor.row = Some(r),
                            Err(err) => editor.status(err),
                        }
                    }
                });
            }
        });
    })
    .response
    .rect
}

#[allow(clippy::too_many_arguments)]
fn inspector(
    root: &mut egui::Ui,
    editor: &mut Editor,
    store: &mut SpawnStore,
    game: &GameData,
    brush: &mut Brush,
    clock: &mut WorldClock,
    speed: &mut FlySpeed,
    paths: Option<&mut crate::paths::AiPaths>,
) -> egui::Rect {
    egui::Panel::right("inspector").resizable(true).default_size(310.0).show(root, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            match editor.mode {
                Mode::Terrain => terrain_panel(ui, brush),
                Mode::Place => {
                    ui.heading("Place");
                    match editor.place {
                        Some((kind, id)) => {
                            let table = if kind == Kind::Npc { Table::Npcs } else { Table::Doodads };
                            ui.label(format!("{} {id}: {}", kind.label(), game.name(table, id).unwrap_or_default()));
                            ui.small("Click the ground to place. Shift-click keeps placing, Esc stops.");
                        }
                        None => {
                            ui.label("Pick an NPC or doodad in the browser.");
                        }
                    }
                }
                Mode::Select | Mode::Objects | Mode::Paths | Mode::Housing => {}
            }
            ui.separator();
            selection_panel(ui, editor, store, game, paths);
            ui.separator();
            ui.collapsing("View", |ui| {
                ui.add(egui::Slider::new(&mut editor.view_radius, 50.0..=600.0).text("spawn radius (m)"));
                ui.checkbox(&mut editor.show_npcs, "NPCs");
                ui.checkbox(&mut editor.show_doodads, "Doodads");
                ui.checkbox(&mut editor.fog, "Fog").on_hover_text("The zone's time of day fog. Some instances (arche_mall_world, instance_hadir_farm) are authored with a dense one.");
                ui.checkbox(&mut editor.snap_to_ground, "Snap moved spawns to the ground");
                ui.add(egui::Slider::new(&mut clock.hour, 0.0..=24.0).text("time of day"));
                ui.add(egui::Slider::new(&mut speed.0, 2.0..=2000.0).logarithmic(true).text("fly speed (m/s)"));
            });
            ui.collapsing("Controls", |ui| {
                ui.small(
                    "Right mouse: look · WASD/Space/Q/E: fly (Shift fast)\n\
                     Left click: select · drag: move · [ ]: turn 15° · Shift+wheel: turn 5°\n\
                     Del: delete · Ctrl+D: duplicate · F: focus · Ctrl+Z/Y: undo/redo\n\
                     1-5: select/place/terrain/objects/paths · Ctrl+S: save all · F5: play\n\
                     M: teleport map · top-right gizmo: drag to orbit, click an axis to look along it",
                );
            });
        });
    })
    .response
    .rect
}

fn terrain_panel(ui: &mut egui::Ui, brush: &mut Brush) {
    ui.heading("Terrain brush");
    ui.horizontal_wrapped(|ui| {
        for (k, label) in [
            (BrushKind::Raise, "Raise"),
            (BrushKind::Lower, "Lower"),
            (BrushKind::Smooth, "Smooth"),
            (BrushKind::Flatten, "Flatten"),
            (BrushKind::Paint, "Paint"),
            (BrushKind::Hole, "Hole"),
            (BrushKind::Tint, "Tint"),
        ] {
            if ui.selectable_label(brush.kind == k, label).clicked() {
                brush.kind = k;
            }
        }
    });
    ui.add(egui::Slider::new(&mut brush.radius, 1.0..=200.0).logarithmic(true).text("radius (m)"));
    match brush.kind {
        BrushKind::Paint => {
            let cell = brush.palette_cell.map(|(x, y)| format!("{x:03}_{y:03}")).unwrap_or_else(|| "-".into());
            ui.label(format!("Surface types of cell {cell}"));
            let mut pick = None;
            egui::ScrollArea::vertical().id_salt("palette").max_height(320.0).show(ui, |ui| {
                for e in &brush.palette {
                    ui.horizontal(|ui| {
                        match e.thumb {
                            Some(id) => {
                                ui.add(egui::Image::new(egui::load::SizedTexture::new(id, [32.0, 32.0])));
                            }
                            None => {
                                ui.add_sized([32.0, 32.0], egui::Label::new("…"));
                            }
                        }
                        let selected = brush.paint.as_deref() == Some(e.material.as_str());
                        if ui.selectable_label(selected, format!("{:>2}  {}", e.slot, e.name)).on_hover_text(&e.material).clicked() {
                            pick = Some(e.material.clone());
                        }
                    });
                }
            });
            if let Some(m) = pick {
                brush.paint = Some(m);
            }
            if let Some(m) = &brush.paint {
                ui.small(format!("painting {}", m.rsplit('/').next().unwrap_or(m)));
            }
            ui.small("Cells without this surface type keep theirs (each cell has its own list of up to 31).");
        }
        BrushKind::Tint => {
            ui.horizontal(|ui| {
                ui.color_edit_button_rgb(&mut brush.tint);
                ui.label("cover colour (Alt+click picks the ground's)");
            });
            ui.add(egui::Slider::new(&mut brush.strength, 0.2..=40.0).logarithmic(true).text("strength"));
            ui.small("Tints the baked cover colour (cover.ctc), the base the surface layers are blended on.");
        }
        BrushKind::Hole => {
            ui.checkbox(&mut brush.erase, "fill holes (or hold Shift)");
            ui.small("Holes cut the terrain mesh and let the player fall through (Play).");
        }
        _ => {
            ui.add(egui::Slider::new(&mut brush.strength, 0.2..=40.0).logarithmic(true).text("strength"));
            ui.small("Flatten keeps the height where the stroke began.");
        }
    }
    ui.small(format!("Hold the left mouse button on the ground. Ctrl+Z undoes a stroke. Last stroke frame {:.1} ms.", brush.last_ms));
    ui.small("Saved as heightmap.dat (heights, surface types, holes) and cover.ctc (tint) into the pak overlay (the editor and the client read it through AA_PAK_OVERLAY).");
}

fn selection_panel(ui: &mut egui::Ui, editor: &mut Editor, store: &mut SpawnStore, game: &GameData, paths: Option<&mut crate::paths::AiPaths>) {
    ui.heading("Selection");
    let Some(i) = editor.selected.filter(|&i| i < store.spawns.len() && !store.spawns[i].now.deleted) else {
        ui.label("Nothing selected. Click a spawn marker or model.");
        return;
    };
    let kind = store.spawns[i].kind;
    let table = if kind == Kind::Npc { Table::Npcs } else { Table::Doodads };
    let mut p = store.spawns[i].now;
    let file = store.files[store.spawns[i].file].path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    ui.label(egui::RichText::new(format!("{} {}", kind.label(), game.name(table, p.unit_id).unwrap_or_else(|| "?".into()))).strong());
    ui.small(format!("in {file}{}", if store.spawns[i].changed() { " · changed" } else { "" }));
    let model = match kind {
        Kind::Npc => game.entry(Table::Npcs, p.unit_id).map(|e| format!("model id {}", e.model_id)),
        Kind::Doodad => game.entry(Table::Doodads, p.unit_id).map(|e| e.model.clone()),
    };
    if let Some(m) = model {
        ui.small(m);
    }
    let before = p;
    egui::Grid::new("placement").num_columns(2).show(ui, |ui| {
        ui.label("UnitId");
        ui.add(egui::DragValue::new(&mut p.unit_id).speed(1.0));
        ui.end_row();
        for (label, v, step) in [("X", &mut p.pos.x, 0.1), ("Y", &mut p.pos.y, 0.1), ("Z", &mut p.pos.z, 0.05)] {
            ui.label(label);
            ui.add(egui::DragValue::new(v).speed(step).max_decimals(3));
            ui.end_row();
        }
        ui.label("Yaw°");
        ui.add(egui::DragValue::new(&mut p.yaw).speed(1.0).range(-180.0..=180.0).max_decimals(2));
        ui.end_row();
        ui.label("Pitch°");
        ui.add(egui::DragValue::new(&mut p.pitch).speed(0.5).range(-90.0..=90.0).max_decimals(2));
        ui.end_row();
        ui.label("Roll°");
        ui.add(egui::DragValue::new(&mut p.roll).speed(0.5).range(-180.0..=180.0).max_decimals(2));
        ui.end_row();
        if kind == Kind::Doodad {
            ui.label("Scale");
            ui.add(egui::DragValue::new(&mut p.scale).speed(0.01).range(0.05..=20.0).max_decimals(3));
            ui.end_row();
        }
    });
    if p != before {
        store.apply_continued(vec![(i, p)]);
    }
    if kind == Kind::Npc
        && let Some(paths) = paths
    {
        crate::paths::spawn_section(ui, editor, store, paths, i);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button("Focus (F)").clicked() {
            editor.requests.push(Request::Focus(p.pos));
        }
        if ui.button("Duplicate").clicked() {
            let mut copy = p;
            copy.pos.x += 2.0;
            let index = store.add(kind, copy);
            editor.selected = Some(index);
        }
        if ui.button("Delete").clicked() {
            store.apply(vec![(i, crate::spawns::Placement { deleted: true, ..p })]);
            editor.selected = None;
        }
        if ui.button("Edit template").clicked() {
            match RowEdit::open(game, table, p.unit_id) {
                Ok(r) => editor.row = Some(r),
                Err(e) => editor.status(e),
            }
        }
    });
}

fn row_editor(ctx: &egui::Context, editor: &mut Editor, game: &mut GameData, store: &SpawnStore, preview: &mut ModelPreview) {
    let Some(mut row) = editor.row.take() else { return };
    let mut open = true;
    let mut status = None;
    let mut reopen: Option<(Table, u32)> = None;
    let mut focus = None;
    egui::Window::new(format!("{} {} — database row", row.table.label(), row.id))
        .open(&mut open)
        .default_size([460.0, 560.0])
        .default_pos([340.0, 80.0])
        .show(ctx, |ui| {
            ui.small(format!("{} · {}", game.path, if game.writable { "writable" } else { "read-only" }));
            egui::Grid::new("texts").num_columns(2).show(ui, |ui| {
                ui.label("Name (en)");
                ui.text_edit_singleline(&mut row.name);
                ui.end_row();
                if let Some(d) = row.description.as_mut() {
                    ui.label("Description (en)");
                    ui.text_edit_multiline(d);
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                let can = game.writable && row.changed();
                if ui.add_enabled(can, egui::Button::new("Save to database")).clicked() {
                    let mut texts = Vec::new();
                    if row.name != row.original_name {
                        texts.push(("name".to_string(), row.name.clone()));
                    }
                    if row.description != row.original_description {
                        texts.push(("description".to_string(), row.description.clone().unwrap_or_default()));
                    }
                    match game.update_row(row.table, row.id, &row.fields, &texts) {
                        Ok(n) => {
                            status = Some(format!("{} {}: {n} value(s) written (restart the server to load them)", row.table.label(), row.id));
                            reopen = Some((row.table, row.id));
                        }
                        Err(e) => status = Some(format!("save failed: {e:#}")),
                    }
                }
                if ui.add_enabled(row.changed(), egui::Button::new("Revert")).clicked() {
                    reopen = Some((row.table, row.id));
                }
                if ui.add_enabled(game.writable, egui::Button::new("Clone as new")).clicked() {
                    match game.clone_row(row.table, row.id) {
                        Ok(id) => {
                            status = Some(format!("{} {} cloned to {id}", row.table.label(), row.id));
                            reopen = Some((row.table, id));
                        }
                        Err(e) => status = Some(format!("clone failed: {e:#}")),
                    }
                }
                if ui.button("Preview").on_hover_text("Show this row in the model preview").clicked() {
                    preview.request = Some(Subject::of_row(row.table, row.id));
                }
                ui.checkbox(&mut row.confirm_delete, "");
                if ui.add_enabled(game.writable && row.confirm_delete, egui::Button::new("Delete row")).on_hover_text("Tick the box first").clicked() {
                    match game.delete_row(row.table, row.id) {
                        Ok(()) => status = Some(format!("{} {} deleted", row.table.label(), row.id)),
                        Err(e) => status = Some(format!("delete failed: {e:#}")),
                    }
                    row.id = 0;
                }
            });
            if row.table == Table::Quests {
                ui.collapsing("Quest steps (quest_components)", |ui| {
                    for (id, kind, npc, name) in game.quest_components(row.id) {
                        ui.horizontal(|ui| {
                            ui.monospace(format!("{id:>6} {:<8}", component_kind(kind)));
                            if npc != 0 {
                                ui.label(format!("NPC {npc} {name}"));
                                let spawn = store.spawns.iter().find(|s| s.kind == Kind::Npc && !s.now.deleted && s.now.unit_id == npc);
                                if let Some(s) = spawn
                                    && ui.small_button("go to").clicked() {
                                        focus = Some(s.now.pos);
                                    }
                            }
                        });
                    }
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Filter columns");
                ui.text_edit_singleline(&mut row.filter);
            });
            let filter = row.filter.to_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("fields").num_columns(2).striped(true).show(ui, |ui| {
                    for f in row.fields.iter_mut().filter(|f| filter.is_empty() || f.column.to_lowercase().contains(&filter)) {
                        let label = if f.text != f.original {
                            egui::RichText::new(&f.column).color(egui::Color32::from_rgb(255, 170, 60))
                        } else {
                            egui::RichText::new(&f.column)
                        };
                        ui.label(label);
                        let edit = egui::TextEdit::singleline(&mut f.text).desired_width(240.0).interactive(f.column != "id");
                        let edit = if f.null { edit.hint_text("NULL") } else { edit };
                        ui.add(edit);
                        ui.end_row();
                    }
                });
            });
        });
    if let Some(s) = status {
        editor.status(s);
    }
    if let Some(p) = focus {
        editor.requests.push(Request::Focus(p));
    }
    if let Some((table, id)) = reopen {
        match RowEdit::open(game, table, id) {
            Ok(r) => row = r,
            Err(e) => editor.status(e),
        }
    }
    if open && row.id != 0 {
        editor.row = Some(row);
    }
}

/// Names over the hovered and selected spawns.
fn labels(ctx: &egui::Context, editor: &Editor, store: &SpawnStore, game: &GameData, cursor: &Cursor, cam: &Query<(&Camera, &GlobalTransform), With<EditorCamera>>) {
    let Ok((camera, t)) = cam.single() else { return };
    let painter = ctx.layer_painter(egui::LayerId::background());
    let scale = ctx.pixels_per_point();
    for (i, strong) in [(cursor.hover, false), (editor.selected, true)] {
        let Some(s) = i.and_then(|i| store.spawns.get(i)).filter(|s| !s.now.deleted) else { continue };
        let table = if s.kind == Kind::Npc { Table::Npcs } else { Table::Doodads };
        let Ok(screen) = camera.world_to_viewport(t, cry_to_bevy(s.now.pos + Vec3::Z * 2.6)) else { continue };
        let text = format!("{} {} {}", s.kind.label(), s.now.unit_id, game.name(table, s.now.unit_id).unwrap_or_default());
        let pos = egui::pos2(screen.x / scale, screen.y / scale);
        let color = if strong { egui::Color32::from_rgb(255, 200, 60) } else { egui::Color32::WHITE };
        painter.text(pos + egui::vec2(1.0, 1.0), egui::Align2::CENTER_BOTTOM, &text, egui::FontId::proportional(14.0), egui::Color32::BLACK);
        painter.text(pos, egui::Align2::CENTER_BOTTOM, &text, egui::FontId::proportional(14.0), color);
    }
}
