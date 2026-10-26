//! The teleport window (M): a map of the world to click on, and a searchable list of places to jump to.
//!
//! * The map is a shaded relief of the terrain heights (one pixel per 32 m), built in the background the first time
//!   the window opens. Drag to pan, wheel to zoom, click to teleport there.
//! * Places: the zones (from the sector ownership of `world.xml`, named after their zone group in the game
//!   database), the server's respawn, recall and world gate points (`Data/Portal/*.json`) and start point, your
//!   bookmarks (`data/local/editor/bookmarks.json`, per world), and NPC / doodad spawns by name.
//! * Coordinates: `x, y` (ground) or `x, y, z`, as the status bar shows them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aa_world::{CELL_SIZE, TerrainQuery, bevy_to_cry};
use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use serde::{Deserialize, Serialize};

use crate::camera::EditorCamera;
use crate::db::{GameData, Table};
use crate::spawns::{Kind, SpawnStore};
use crate::{Editor, Request};

/// Map pixels per cell (32 m each).
const PX: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaceKind {
    Bookmark,
    Zone,
    Start,
    Portal,
}

impl PlaceKind {
    fn label(self) -> &'static str {
        match self {
            PlaceKind::Bookmark => "bookmark",
            PlaceKind::Zone => "zone",
            PlaceKind::Start => "start",
            PlaceKind::Portal => "portal",
        }
    }

    fn color(self) -> egui::Color32 {
        match self {
            PlaceKind::Bookmark => egui::Color32::from_rgb(90, 220, 240),
            PlaceKind::Zone => egui::Color32::WHITE,
            PlaceKind::Start => egui::Color32::from_rgb(255, 120, 220),
            PlaceKind::Portal => egui::Color32::from_rgb(250, 210, 80),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Place {
    pub name: String,
    pub kind: PlaceKind,
    /// Cry space; `z` NaN: the ground there.
    pub pos: Vec3,
}

/// What the background build makes.
pub struct Built {
    /// RGBA, `w` x `h`, row 0 at the north edge.
    pub rgba: Vec<u8>,
    pub w: usize,
    pub h: usize,
    pub places: Vec<Place>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Bookmark {
    name: String,
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Resource)]
pub struct Teleport {
    pub open: bool,
    job: Option<(Arc<AtomicUsize>, usize, Arc<Mutex<Option<Built>>>)>,
    places: Vec<Place>,
    map: Option<(egui::TextureHandle, f32, f32)>,
    /// Map view: centre (cry XY) and metres per point (0: fit on the next frame).
    center: Vec2,
    mpp: f32,
    coords: String,
    search: String,
    show: [bool; 5],
    bookmark_name: String,
    bookmarks_file: PathBuf,
    bookmarks: BTreeMap<String, Vec<Bookmark>>,
    /// Spawn search: (query, store generation) -> spawn indices.
    found: Option<(String, u64, Vec<usize>)>,
    hover: Option<Vec3>,
}

impl Default for Teleport {
    fn default() -> Self {
        let bookmarks_file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/local/editor/bookmarks.json");
        let bookmarks = std::fs::read_to_string(&bookmarks_file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Teleport {
            open: crate::flag("--teleport"),
            job: None,
            places: Vec::new(),
            map: None,
            center: Vec2::ZERO,
            mpp: 0.0,
            coords: String::new(),
            search: String::new(),
            show: [true; 5],
            bookmark_name: String::new(),
            bookmarks_file,
            bookmarks,
            found: None,
            hover: None,
        }
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Teleport>().add_systems(Update, keys).add_systems(EguiPrimaryContextPass, window.after(crate::ui::panels));
}

fn keys(keys: Res<ButtonInput<KeyCode>>, egui: Res<EguiWantsInput>, mut editor: ResMut<Editor>, mut tp: ResMut<Teleport>) {
    if let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::OpenTeleport)) {
        editor.requests.remove(i);
        tp.open = !tp.open;
    }
    if keys.just_pressed(KeyCode::KeyM) && !egui.wants_any_keyboard_input() && !editor.playing {
        tp.open = !tp.open;
    }
}

/// Parses `x, y` or `x, y, z` (spaces or commas).
pub fn parse_coords(text: &str) -> Option<Vec3> {
    let v: Vec<f32> = text.split([',', ' ', ';']).filter(|s| !s.is_empty()).map(|s| s.parse().ok()).collect::<Option<_>>()?;
    match v[..] {
        [x, y] => Some(Vec3::new(x, y, f32::NAN)),
        [x, y, z] => Some(Vec3::new(x, y, z)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// background build

fn land_color(h: f32, ocean: f32, light: f32) -> [u8; 3] {
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    let e = h - ocean;
    let c = if e < 0.0 {
        let d = (-e / 80.0).clamp(0.0, 1.0);
        let c = lerp([70.0, 135.0, 185.0], [18.0, 40.0, 92.0], d);
        return c.map(|v| v as u8);
    } else {
        const STOPS: [(f32, [f32; 3]); 5] =
            [(0.0, [196.0, 184.0, 130.0]), (12.0, [104.0, 150.0, 76.0]), (220.0, [72.0, 112.0, 58.0]), (520.0, [128.0, 112.0, 84.0]), (900.0, [225.0, 225.0, 225.0])];
        let mut c = STOPS[4].1;
        for w in STOPS.windows(2) {
            if e < w[1].0 {
                c = lerp(w[0].1, w[1].1, (e - w[0].0) / (w[1].0 - w[0].0));
                break;
            }
        }
        c
    };
    c.map(|v| (v * light).clamp(0.0, 255.0) as u8)
}

/// The relief map: heights every 32 m, shaded from the north-west.
fn relief(world: &str, cells: (i32, i32), ocean: f32, progress: &AtomicUsize) -> (Vec<u8>, usize, usize) {
    let (cx_n, cy_n) = (cells.0.max(1) as usize, cells.1.max(1) as usize);
    let (w, h) = (cx_n * PX, cy_n * PX);
    // heights[(row from the south) * w + col]
    let heights: Vec<f32> = {
        let threads = 4.min(cy_n);
        let rows: Vec<Vec<(usize, Vec<f32>)>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    s.spawn(move || {
                        let q = TerrainQuery::with_capacity(world, 2);
                        let mut out = Vec::new();
                        for cy in (t..cy_n).step_by(threads) {
                            let mut band = vec![f32::NAN; w * PX];
                            for cx in 0..cx_n {
                                if let Some(c) = q.cell(cx as i32, cy as i32) {
                                    let step = (aa_world::heightmap::GRID - 1) / PX;
                                    for iy in 0..PX {
                                        for ix in 0..PX {
                                            band[iy * w + cx * PX + ix] = c.at(ix * step, iy * step);
                                        }
                                    }
                                }
                                progress.fetch_add(1, Ordering::Relaxed);
                            }
                            out.push((cy, band));
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        let mut all = vec![f32::NAN; w * h];
        for (cy, band) in rows.into_iter().flatten() {
            all[cy * PX * w..(cy + 1) * PX * w].copy_from_slice(&band);
        }
        all
    };
    let at = |x: usize, y: usize| heights[y.min(h - 1) * w + x.min(w - 1)];
    let light_dir = Vec3::new(-1.0, 1.0, 1.4).normalize();
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let z = at(x, y);
            let o = ((h - 1 - y) * w + x) * 4;
            if z.is_nan() {
                rgba[o..o + 4].copy_from_slice(&[24, 24, 28, 255]);
                continue;
            }
            let nan0 = |v: f32| if v.is_nan() { z } else { v };
            let dx = (nan0(at(x + 1, y)) - nan0(at(x.saturating_sub(1), y))) / 64.0;
            let dy = (nan0(at(x, y + 1)) - nan0(at(x, y.saturating_sub(1)))) / 64.0;
            let n = Vec3::new(-dx * 2.0, -dy * 2.0, 1.0).normalize();
            let light = if z < ocean { 1.0 } else { 0.55 + 0.6 * n.dot(light_dir).max(0.0) };
            let [r, g, b] = land_color(z, ocean, light);
            rgba[o..o + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    (rgba, w, h)
}

/// Zone centres: the owned sector nearest to the middle of each zone group's sectors.
fn zones(world: &aa_world::World, db: &Path) -> Vec<Place> {
    // zone id -> (group key, display name)
    let mut groups: HashMap<u16, (i64, String)> = HashMap::new();
    if let Ok(c) = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
        let sql = "SELECT z.zone_key, z.group_id, COALESCE(NULLIF(l.en_us, ''), g.name) FROM zones z \
                   LEFT JOIN zone_groups g ON g.id = z.group_id \
                   LEFT JOIN localized_texts l ON l.tbl_name = 'zone_groups' AND l.tbl_column_name = 'display_text' AND l.idx = g.id";
        if let Ok(mut st) = c.prepare(sql) {
            let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<String>>(2)?)));
            for (key, group, name) in rows.into_iter().flatten().flatten() {
                if let (Some(g), Some(n)) = (group, name) {
                    groups.insert(key as u16, (g, n));
                }
            }
        }
    }
    let filler = format!("{}_", world.name);
    let mut sectors: HashMap<(i64, String), Vec<Vec2>> = HashMap::new();
    for (&(cx, cy), owners) in &world.sector_zones {
        for (i, &id) in owners.iter().enumerate() {
            if id == 0 {
                continue;
            }
            let key = match groups.get(&id) {
                Some((g, n)) => (*g, n.clone()),
                None => match world.zones.iter().find(|z| z.id == id) {
                    Some(z) if !z.name.starts_with(&filler) => (-(id as i64), z.name.clone()),
                    _ => continue,
                },
            };
            let (sx, sy) = ((i / 16) as f32, (i % 16) as f32);
            let p = Vec2::new(cx as f32 * CELL_SIZE + (sx + 0.5) * 64.0, cy as f32 * CELL_SIZE + (sy + 0.5) * 64.0);
            sectors.entry(key).or_default().push(p);
        }
    }
    // zones without a zone group carry only their internal names: list them only in worlds without named groups
    if sectors.keys().any(|(g, _)| *g >= 0) {
        sectors.retain(|(g, _), _| *g >= 0);
    }
    let mut out: Vec<Place> = sectors
        .into_iter()
        .filter(|(_, v)| v.len() >= 4)
        .map(|((_, name), v)| {
            let mid = v.iter().copied().sum::<Vec2>() / v.len() as f32;
            let best = v.iter().copied().min_by(|a, b| a.distance_squared(mid).total_cmp(&b.distance_squared(mid))).unwrap_or(mid);
            Place { name, kind: PlaceKind::Zone, pos: best.extend(f32::NAN) }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The server's respawn / recall / world gate points that lie in this world (on its ground).
fn portals(world: &aa_world::World, data: &[PathBuf], terrain: &TerrainQuery) -> Vec<Place> {
    let mut out: Vec<Place> = Vec::new();
    let Some(dir) = data.iter().map(|d| d.join("Portal")).find(|d| d.is_dir()) else { return out };
    for file in ["respawns.json", "worldgates.json", "recalls.json"] {
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else { continue };
        let text = crate::jsonc::strip_comments(text.trim_start_matches('\u{feff}'));
        let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for e in list {
            let (Some(x), Some(y), Some(z)) = (e["X"].as_f64(), e["Y"].as_f64(), e["Z"].as_f64()) else { continue };
            let p = Vec3::new(x as f32, y as f32, z as f32);
            let on_ground = terrain.height_at(p.x, p.y).is_some_and(|h| (h - p.z).abs() < 25.0);
            if world.sector_zone_at(p).is_none() || !on_ground {
                continue;
            }
            let name = e["Name"].as_str().unwrap_or("?").trim_start_matches("Respawn: ").to_string();
            if out.iter().any(|q| q.name == name && q.pos.distance(p) < 50.0) {
                continue;
            }
            out.push(Place { name, kind: PlaceKind::Portal, pos: p });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn build(world: String, db: PathBuf, data: Vec<PathBuf>, progress: Arc<AtomicUsize>) -> Built {
    let mut w = aa_world::World::load(&world).unwrap_or_default();
    if w.name.is_empty() {
        w.name = world.clone();
    }
    let _ = w.load_sector_zones();
    let (rgba, mw, mh) = relief(&world, (w.cells_x, w.cells_y), w.ocean_level, &progress);
    let terrain = TerrainQuery::with_capacity(&world, 8);
    let mut places = Vec::new();
    if let Some(p) = crate::worlds::server_start(&world, &data) {
        places.push(Place { name: "Server start point".into(), kind: PlaceKind::Start, pos: p });
    }
    places.extend(zones(&w, &db));
    places.extend(portals(&w, &data, &terrain));
    Built { rgba, w: mw, h: mh, places }
}

// ---------------------------------------------------------------------------------------------------------------
// window

impl Teleport {
    fn save_bookmarks(&self) -> Result<(), String> {
        if let Some(d) = self.bookmarks_file.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&self.bookmarks).map_err(|e| e.to_string())?;
        std::fs::write(&self.bookmarks_file, text).map_err(|e| e.to_string())
    }

    fn start(&mut self, editor: &Editor) {
        let total = aa_world::World::load(&editor.world).map(|w| (w.cells_x.max(1) * w.cells_y.max(1)) as usize).unwrap_or(1);
        let progress = Arc::new(AtomicUsize::new(0));
        let slot = Arc::new(Mutex::new(None));
        let (world, db, data) = (editor.world.clone(), editor.paths.db.clone(), crate::worlds::data_dirs(&editor.paths.spawn_dir));
        let (p, s) = (progress.clone(), slot.clone());
        std::thread::spawn(move || {
            let t = std::time::Instant::now();
            let built = build(world.clone(), db, data, p);
            info!("teleport map of {world}: {}x{} px, {} places, {:.1} s", built.w, built.h, built.places.len(), t.elapsed().as_secs_f32());
            *s.lock().unwrap_or_else(|e| e.into_inner()) = Some(built);
        });
        self.job = Some((progress, total, slot));
    }
}

fn window(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut tp: ResMut<Teleport>,
    store: Res<SpawnStore>,
    game: Res<GameData>,
    cam: Query<&Transform, With<EditorCamera>>,
) -> Result {
    if !tp.open || editor.playing {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    // the background build
    if tp.map.is_none() && tp.job.is_none() {
        tp.start(&editor);
    }
    let done = tp.job.as_ref().and_then(|(_, _, slot)| slot.lock().ok()?.take());
    if let Some(b) = done {
        let image = egui::ColorImage::from_rgba_unmultiplied([b.w, b.h], &b.rgba);
        let tex = ctx.load_texture("teleport map", image, egui::TextureOptions::LINEAR);
        tp.map = Some((tex, (b.w / PX) as f32 * CELL_SIZE, (b.h / PX) as f32 * CELL_SIZE));
        tp.places = b.places;
        tp.job = None;
    }
    let camera = cam.single().ok().map(|t| (bevy_to_cry(t.translation), bevy_to_cry(t.forward().as_vec3())));
    let world = editor.world.clone();

    let mut open = tp.open;
    let mut go: Option<(Vec3, String)> = None;
    egui::Window::new(format!("Teleport — {world}"))
        .open(&mut open)
        .default_size([980.0, 640.0])
        .resizable(true)
        .show(&ctx, |ui| {
            // coordinates and bookmarks
            ui.horizontal(|ui| {
                ui.label("Go to");
                let r = ui.add(egui::TextEdit::singleline(&mut tp.coords).hint_text("x, y  or  x, y, z").desired_width(170.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Go").clicked() || enter {
                    match parse_coords(&tp.coords) {
                        Some(p) => go = Some((p, tp.coords.clone())),
                        None => editor.status("teleport: type x, y or x, y, z"),
                    }
                }
                ui.separator();
                if let Some((p, _)) = camera {
                    ui.monospace(format!("camera {:.0}, {:.0}, {:.0}", p.x, p.y, p.z));
                    if ui.small_button("copy").clicked() {
                        ctx.copy_text(format!("{:.1}, {:.1}, {:.1}", p.x, p.y, p.z));
                    }
                }
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut tp.bookmark_name).hint_text("bookmark name").desired_width(130.0));
                if ui.button("★ Bookmark camera").clicked()
                    && let Some((p, _)) = camera
                {
                    let n = tp.bookmarks.get(&world).map_or(0, Vec::len) + 1;
                    let name = if tp.bookmark_name.trim().is_empty() { format!("Bookmark {n}") } else { tp.bookmark_name.trim().to_string() };
                    tp.bookmarks.entry(world.clone()).or_default().push(Bookmark { name: name.clone(), x: p.x, y: p.y, z: p.z });
                    tp.bookmark_name.clear();
                    match tp.save_bookmarks() {
                        Ok(()) => editor.status(format!("bookmarked {name}")),
                        Err(e) => editor.status(format!("cannot save bookmarks: {e}")),
                    }
                }
            });
            ui.separator();

            let mut places: Vec<Place> = tp
                .bookmarks
                .get(&world)
                .into_iter()
                .flatten()
                .map(|b| Place { name: b.name.clone(), kind: PlaceKind::Bookmark, pos: Vec3::new(b.x, b.y, b.z) })
                .collect();
            places.extend(tp.places.iter().cloned());

            // the list
            let mut delete_bookmark = None;
            egui::Panel::right("teleport places").resizable(true).default_size(300.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.text_edit_singleline(&mut tp.search);
                });
                ui.horizontal_wrapped(|ui| {
                    for (i, label) in ["Bookmarks", "Zones", "Start", "Portals", "NPCs / doodads"].into_iter().enumerate() {
                        ui.checkbox(&mut tp.show[i], label);
                    }
                });
                ui.separator();
                let needle = tp.search.trim().to_lowercase();
                tp.hover = None;
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    let mut bookmark_i = 0;
                    for p in &places {
                        let i = p.kind as usize;
                        let bi = bookmark_i;
                        if p.kind == PlaceKind::Bookmark {
                            bookmark_i += 1;
                        }
                        if !tp.show[i] || !needle.is_empty() && !p.name.to_lowercase().contains(&needle) {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.colored_label(p.kind.color(), "●");
                            let r = ui.selectable_label(false, &p.name).on_hover_text(format!("{} · {:.0}, {:.0}", p.kind.label(), p.pos.x, p.pos.y));
                            if r.hovered() {
                                tp.hover = Some(p.pos);
                            }
                            if r.clicked() {
                                go = Some((p.pos, p.name.clone()));
                            }
                            if p.kind == PlaceKind::Bookmark && ui.small_button("✖").on_hover_text("Delete the bookmark").clicked() {
                                delete_bookmark = Some(bi);
                            }
                        });
                    }
                    // spawns by name
                    if tp.show[4] && needle.chars().count() >= 2 {
                        let stale = tp.found.as_ref().is_none_or(|(q, g, _)| *q != needle || *g != store.generation);
                        if stale {
                            let hits: Vec<usize> = store
                                .spawns
                                .iter()
                                .enumerate()
                                .filter(|(_, s)| !s.now.deleted)
                                .filter(|(_, s)| {
                                    let t = if s.kind == Kind::Npc { Table::Npcs } else { Table::Doodads };
                                    s.now.unit_id.to_string() == needle || game.name(t, s.now.unit_id).is_some_and(|n| n.to_lowercase().contains(&needle))
                                })
                                .map(|(i, _)| i)
                                .take(300)
                                .collect();
                            tp.found = Some((needle.clone(), store.generation, hits));
                        }
                        let hits = tp.found.as_ref().map(|f| f.2.clone()).unwrap_or_default();
                        ui.separator();
                        ui.label(format!("{} spawn(s){}", hits.len(), if hits.len() >= 300 { " (first 300)" } else { "" }));
                        for i in hits {
                            let s = &store.spawns[i];
                            let t = if s.kind == Kind::Npc { Table::Npcs } else { Table::Doodads };
                            let name = game.name(t, s.now.unit_id).unwrap_or_default();
                            let r = ui
                                .selectable_label(false, format!("{} {}  {name}", s.kind.label(), s.now.unit_id))
                                .on_hover_text(format!("{:.0}, {:.0}, {:.0}", s.now.pos.x, s.now.pos.y, s.now.pos.z));
                            if r.hovered() {
                                tp.hover = Some(s.now.pos);
                            }
                            if r.clicked() {
                                editor.selected = Some(i);
                                editor.requests.push(Request::Focus(s.now.pos));
                                editor.status(format!("{} {} selected", s.kind.label(), s.now.unit_id));
                            }
                        }
                    }
                });
            });
            if let Some(i) = delete_bookmark {
                if let Some(v) = tp.bookmarks.get_mut(&world)
                    && i < v.len()
                {
                    v.remove(i);
                }
                if let Err(e) = tp.save_bookmarks() {
                    editor.status(format!("cannot save bookmarks: {e}"));
                }
            }

            // the map
            let Some((tex, ww, wh)) = tp.map.clone() else {
                let (n, total) = tp.job.as_ref().map(|(p, t, _)| (p.load(Ordering::Relaxed), *t)).unwrap_or((0, 1));
                ui.centered_and_justified(|ui| {
                    ui.add(egui::ProgressBar::new(n as f32 / total.max(1) as f32).text(format!("building the map… {n} / {total} cells")));
                });
                return;
            };
            let (rect, resp) = ui.allocate_exact_size(ui.available_size().max(egui::vec2(200.0, 200.0)), egui::Sense::click_and_drag());
            if tp.mpp <= 0.0 {
                tp.mpp = (ww / rect.width()).max(wh / rect.height()) * 1.02;
                tp.center = Vec2::new(ww, wh) / 2.0;
            }
            let (center, mpp) = (tp.center, tp.mpp);
            let to_screen = |p: Vec2| rect.center() + egui::vec2((p.x - center.x) / mpp, -(p.y - center.y) / mpp);
            let from_screen = |s: egui::Pos2| Vec2::new(center.x + (s.x - rect.center().x) * mpp, center.y - (s.y - rect.center().y) * mpp);
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(14, 16, 22));
            painter.image(tex.id(), egui::Rect::from_min_max(to_screen(Vec2::new(0.0, wh)), to_screen(Vec2::new(ww, 0.0))), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);

            // places: dots first, then the labels that fit (zones first, then bookmarks, start, portals)
            let pointer = resp.hover_pos();
            let mut under: Option<(f32, &Place)> = None;
            let visible: Vec<(&Place, egui::Pos2)> = places
                .iter()
                .filter(|p| tp.show[p.kind as usize])
                .map(|p| (p, to_screen(p.pos.truncate())))
                .filter(|(_, s)| rect.expand(40.0).contains(*s))
                .collect();
            for &(p, s) in &visible {
                if p.kind != PlaceKind::Zone {
                    painter.circle(s, 3.0, p.kind.color(), egui::Stroke::new(1.0, egui::Color32::BLACK));
                }
                if let Some(m) = pointer {
                    let d = m.distance(s);
                    if d < 8.0 && under.is_none_or(|(u, _)| d < u) {
                        under = Some((d, p));
                    }
                }
            }
            let mut taken: Vec<egui::Rect> = Vec::new();
            let mut order: Vec<&(&Place, egui::Pos2)> = visible.iter().collect();
            order.sort_by_key(|(p, _)| if p.kind == PlaceKind::Zone { 0 } else { 1 + p.kind as u8 });
            for &&(p, s) in &order {
                let zone = p.kind == PlaceKind::Zone;
                let font = egui::FontId::proportional(if zone { 12.5 } else { 11.0 });
                let galley = painter.layout_no_wrap(p.name.clone(), font, if zone { egui::Color32::WHITE } else { p.kind.color() });
                let at = if zone { s - galley.size() / 2.0 } else { s + egui::vec2(6.0, -galley.size().y / 2.0) };
                let r = egui::Rect::from_min_size(at, galley.size()).expand(2.0);
                if taken.iter().any(|t| t.intersects(r)) {
                    continue;
                }
                taken.push(r);
                painter.rect_filled(r, 3.0, egui::Color32::from_black_alpha(if zone { 110 } else { 150 }));
                painter.galley(at, galley, egui::Color32::WHITE);
            }
            if let Some(h) = tp.hover {
                painter.circle_stroke(to_screen(h.truncate()), 9.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 80, 80)));
            }
            // the camera: a triangle pointing where it looks
            if let Some((p, f)) = camera {
                let s = to_screen(p.truncate());
                let d = egui::vec2(f.x, -f.y).normalized();
                let d = if d.x.is_nan() { egui::vec2(0.0, -1.0) } else { d };
                let side = egui::vec2(-d.y, d.x);
                let tri = vec![s + d * 11.0, s - d * 6.0 + side * 6.0, s - d * 6.0 - side * 6.0];
                painter.add(egui::Shape::convex_polygon(tri, egui::Color32::from_rgb(255, 60, 60), egui::Stroke::new(1.5, egui::Color32::WHITE)));
            }

            // pan, zoom, click
            if resp.dragged() {
                let d = resp.drag_delta();
                tp.center += Vec2::new(-d.x, d.y) * mpp;
            }
            if let Some(m) = resp.hover_pos() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    let before = from_screen(m);
                    tp.mpp = (mpp * (-scroll * 0.002).exp()).clamp(0.2, 400.0);
                    let after = Vec2::new(tp.center.x + (m.x - rect.center().x) * tp.mpp, tp.center.y - (m.y - rect.center().y) * tp.mpp);
                    tp.center += before - after;
                }
                let w = from_screen(m);
                let what = under.map(|(_, p)| format!("{} ({}) · ", p.name, p.kind.label())).unwrap_or_default();
                let line = format!("{what}{:.0}, {:.0} · click to teleport · drag to pan · wheel to zoom", w.x, w.y);
                let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 22.0), rect.max);
                painter.rect_filled(bar, 0.0, egui::Color32::from_black_alpha(160));
                painter.text(bar.left_center() + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, line, egui::FontId::monospace(12.0), egui::Color32::WHITE);
                if resp.clicked() {
                    go = Some(match under {
                        Some((_, p)) => (p.pos, p.name.clone()),
                        None => (w.extend(f32::NAN), format!("{:.0}, {:.0}", w.x, w.y)),
                    });
                }
            }
            // buttons over the map
            let mut b = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)).layout(egui::Layout::top_down(egui::Align::Min)));
            b.horizontal(|ui| {
                if ui.button("Fit world").clicked() {
                    tp.mpp = 0.0;
                }
                if ui.button("Centre on camera").clicked()
                    && let Some((p, _)) = camera
                {
                    tp.center = p.truncate();
                    tp.mpp = tp.mpp.min(4.0);
                }
            });
        });
    tp.open = open;
    if let Some((p, what)) = go {
        editor.requests.push(Request::Teleport(p));
        editor.status(format!("teleport to {what}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates() {
        assert!(parse_coords("15560, 15350").is_some_and(|p| p.x == 15560.0 && p.y == 15350.0 && p.z.is_nan()));
        assert_eq!(parse_coords("1 2 3"), Some(Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!(parse_coords("1"), None);
        assert_eq!(parse_coords("a, b"), None);
    }

    #[test]
    fn colours() {
        assert!(land_color(50.0, 100.0, 1.0)[2] > 100); // water is blue
        assert!(land_color(110.0, 100.0, 1.0)[1] > 120); // lowland is green
        assert_eq!(land_color(2000.0, 100.0, 1.0), [225, 225, 225]); // peaks are snow
    }

    #[test]
    fn main_world_places() {
        if !aa_pak::Pak::global().exists("worlds/main_world/world.g") {
            return;
        }
        let db = PathBuf::from(aa_anim::db::DEFAULT_DB);
        let mut w = aa_world::World::load("main_world").unwrap();
        w.load_sector_zones().unwrap();
        let z = zones(&w, &db);
        let solz = z.iter().find(|p| p.name == "Solzreed Peninsula").expect("Solzreed");
        assert!(w.sector_zone_at(solz.pos.truncate().extend(0.0)).is_some());
        let t = TerrainQuery::new("main_world");
        let p = portals(&w, &crate::worlds::data_dirs(Path::new("x/y")), &t);
        if Path::new(crate::worlds::LANE_DATA).is_dir() {
            assert!(p.len() > 50, "{} portal points", p.len());
        }
    }
}
