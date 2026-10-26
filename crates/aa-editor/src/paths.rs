//! AI paths: the routes NPCs walk, from the AAEmu server's `Data/Path/<name>.path` files (Paths mode, key 5).
//!
//! A route file is one point per line, `action|x|y|z|param` (cry space metres; `action` empty or one of
//! `DisableLoop`, `EnableLoop`, `Speed`, `ActorFlags`; `param` its value), CRLF, as
//! `aaemu-game/src/systems/ai/path_handler.rs` reads it. The server finds a route by file name: an NPC spawn entry's
//! `"FollowPath": "<name>"` ([`crate::spawns`]), or a FollowPath AI command (`ai_commands.cmd_id = 2`, `param2` =
//! name) of a command set that quests and skills run. A name used in several zones may ship as `<name>@<zone>.path`
//! variants; the server takes the one passing closest to the NPC.
//!
//! * The routes near the camera are drawn (green start, red end, arrows along the way); the route of the selected NPC
//!   is drawn in every mode.
//! * Click a point to select it and drag it over the ground. Shift+click the ground adds a point after the selected one
//!   (or at the end). Delete removes the selected point. Ctrl+Z / Ctrl+Y undo / redo.
//! * The panel lists the routes (search, near the camera first) with who uses them, and edits the selected one: name,
//!   points (action, param, position), reverse, close the loop, snap to the ground, walk preview, delete. New routes
//!   start empty or at the selected NPC (which then follows it). Any NPC spawn can be given a route.
//! * Saving (Ctrl+S or the panel) writes the changed files into the path folder, keeping the first overwritten or
//!   removed version as `<name>.path.editor-orig`, and copies them to the mirrors' `Path` folders. The server reads
//!   routes and spawns when it starts.
//!
//! The path folder: `--path-dir <dir>`, else `Data/Path` next to the spawn folder's `Data/Worlds` (the server layout),
//! else a `Path` folder beside the spawn folder (a scratch copy: filled from the lane's files the first time).

use std::path::{Path, PathBuf};

use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::camera::EditorCamera;
use crate::db::{GameData, PathCommand};
use crate::spawns::{Kind, SpawnStore};
use crate::terrain_edit::Heights;
use crate::tools::Cursor;
use crate::{Editor, Mode, Request};

/// The point actions the server knows (an empty action is `None`).
pub const ACTIONS: [&str; 5] = ["", "DisableLoop", "EnableLoop", "Speed", "ActorFlags"];
/// Walk preview speed, m/s (the server's default walk).
const WALK_SPEED: f32 = 3.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Pt {
    pub action: String,
    /// Cry space.
    pub pos: Vec3,
    pub param: String,
    /// The coordinates as the file wrote them and their values: an unmoved axis is written back as it was (`f32`
    /// cannot tell `17741.115` from `17741.116`).
    src: Option<([String; 3], Vec3)>,
}

impl Pt {
    pub fn at(pos: Vec3) -> Pt {
        Pt { action: String::new(), pos, param: String::new(), src: None }
    }
}

#[derive(Clone, Debug)]
pub struct Route {
    pub name: String,
    pub points: Vec<Pt>,
    /// The file stem on disk and its points; `None` for a route not saved yet.
    saved: Option<(String, Vec<Pt>)>,
    pub deleted: bool,
}

impl Route {
    pub fn changed(&self) -> bool {
        match &self.saved {
            None => !self.deleted,
            Some((name, points)) => self.deleted || *name != self.name || *points != self.points,
        }
    }

    /// Length along the points, metres.
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| w[0].pos.distance(w[1].pos)).sum()
    }

    /// Horizontal distance from `p` to the route (its points and segments).
    fn distance_to(&self, p: Vec3) -> f32 {
        let p = p.truncate();
        match self.points.as_slice() {
            [] => f32::INFINITY,
            [a] => a.pos.truncate().distance(p),
            pts => pts
                .windows(2)
                .map(|w| {
                    let (a, b) = (w[0].pos.truncate(), w[1].pos.truncate());
                    let ab = b - a;
                    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                    (a + ab * t).distance(p)
                })
                .fold(f32::INFINITY, f32::min),
        }
    }

    /// The point `d` metres along the route (looping back to the start).
    fn along(&self, d: f32) -> Option<Vec3> {
        let len = self.length();
        if self.points.is_empty() {
            return None;
        }
        if len <= 0.0 {
            return Some(self.points[0].pos);
        }
        let mut d = d.rem_euclid(len);
        for w in self.points.windows(2) {
            let l = w[0].pos.distance(w[1].pos);
            if d <= l {
                return Some(w[0].pos.lerp(w[1].pos, if l > 0.0 { d / l } else { 0.0 }));
            }
            d -= l;
        }
        self.points.last().map(|p| p.pos)
    }
}

/// Parses a `.path` file (lines of exactly five `|` columns, as the server does).
pub fn parse(text: &str) -> Vec<Pt> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.lines()
        .filter_map(|line| {
            let c: Vec<&str> = line.split('|').collect();
            if c.len() != 5 {
                return None;
            }
            let f = |s: &str| s.trim().replace(',', ".").parse::<f32>().unwrap_or(0.0);
            let pos = Vec3::new(f(c[1]), f(c[2]), f(c[3]));
            let src = [c[1].trim().to_string(), c[2].trim().to_string(), c[3].trim().to_string()];
            Some(Pt { action: c[0].trim().to_string(), pos, param: c[4].trim().to_string(), src: Some((src, pos)) })
        })
        .collect()
}

/// The text of a `.path` file.
pub fn write(points: &[Pt]) -> String {
    points
        .iter()
        .map(|p| {
            let axis = |i: usize| match &p.src {
                Some((text, at)) if at[i] == p.pos[i] => text[i].clone(),
                _ => num(p.pos[i]),
            };
            format!("{}|{}|{}|{}|{}\r\n", p.action, axis(0), axis(1), axis(2), p.param)
        })
        .collect()
}

/// Millimetres, written as the shortest text that reads back to the same `f32` (`17741.116`, not `17741.115`),
/// with three decimals like the retail files.
fn num(v: f32) -> String {
    let v = ((v as f64 * 1000.0).round() / 1000.0) as f32;
    let mut s = format!("{v}");
    let decimals = s.split_once('.').map_or(0, |(_, d)| d.len());
    if decimals == 0 {
        s.push('.');
    }
    s.extend(std::iter::repeat_n('0', 3usize.saturating_sub(decimals)));
    s
}

/// A route name must be a plain file stem.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() < 120 && name.chars().all(|c| c.is_ascii_alphanumeric() || "_-@.".contains(c)) && !name.starts_with('.')
}

#[derive(Default)]
struct Drag {
    route: usize,
    point: usize,
    above: f32,
    press: Vec2,
    started: bool,
}

#[derive(Resource)]
pub struct AiPaths {
    pub dir: PathBuf,
    mirrors: Vec<PathBuf>,
    pub routes: Vec<Route>,
    pub selected: Option<usize>,
    pub point: Option<usize>,
    hover: Option<(usize, Option<usize>)>,
    drag: Option<Drag>,
    undo: Vec<(usize, Route)>,
    redo: Vec<(usize, Route)>,
    pub open: bool,
    search: String,
    near_only: bool,
    radius: f32,
    new_name: String,
    rename: String,
    rename_for: Option<usize>,
    /// FollowPath AI commands from the database as edited, loaded on first use; `commands_saved` as read.
    commands: Option<Vec<PathCommand>>,
    commands_saved: Vec<PathCommand>,
    /// Every AI command set (id, name), for adding a command; loaded on first use.
    sets: Option<Vec<(u32, String)>>,
    add_set: String,
    /// A rename also re-points the AI commands that name the old route.
    rename_commands: bool,
    /// Walk preview: metres along the selected route.
    walk: Option<f32>,
    snap: bool,
}

impl AiPaths {
    fn empty(dir: PathBuf, mirrors: Vec<PathBuf>) -> AiPaths {
        AiPaths {
            dir,
            mirrors,
            routes: Vec::new(),
            selected: None,
            point: None,
            hover: None,
            drag: None,
            undo: Vec::new(),
            redo: Vec::new(),
            open: true,
            search: String::new(),
            near_only: true,
            radius: 600.0,
            new_name: String::new(),
            rename: String::new(),
            rename_for: None,
            commands: None,
            commands_saved: Vec::new(),
            sets: None,
            add_set: String::new(),
            rename_commands: true,
            walk: None,
            snap: true,
        }
    }

    /// Reads every `*.path` of `dir`.
    pub fn load(dir: &Path, mirrors: Vec<PathBuf>) -> AiPaths {
        let mut paths = AiPaths::empty(dir.to_path_buf(), mirrors);
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("path")))
            .collect();
        files.sort();
        for f in files {
            let (Some(stem), Ok(bytes)) = (f.file_stem().and_then(|s| s.to_str()), std::fs::read(&f)) else { continue };
            let points = parse(&String::from_utf8_lossy(&bytes));
            paths.routes.push(Route { name: stem.to_string(), saved: Some((stem.to_string(), points.clone())), points, deleted: false });
        }
        info!("ai paths: {} routes in {}", paths.routes.len(), dir.display());
        paths
    }

    /// Changed routes and AI commands.
    pub fn changed_count(&self) -> usize {
        self.routes.iter().filter(|r| r.changed()).count() + self.commands_changed()
    }

    /// AI command rows added, removed or re-pointed since they were read.
    fn commands_changed(&self) -> usize {
        let Some(now) = &self.commands else { return 0 };
        let before = &self.commands_saved;
        before.iter().filter(|b| !now.iter().any(|n| n.id == b.id)).count()
            + now.iter().filter(|n| n.id == 0 || before.iter().any(|b| b.id == n.id && b != *n)).count()
    }

    /// The AI commands that name route `name` (or the zone-less name of an `@zone` variant).
    fn commands_of(&self, name: &str) -> Vec<usize> {
        let Some(c) = &self.commands else { return Vec::new() };
        (0..c.len()).filter(|&k| c[k].route == name || name.starts_with(&format!("{}@", c[k].route))).collect()
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.routes.iter().position(|r| !r.deleted && r.name == name)
    }

    /// The route a spawn's `FollowPath` names: the exact file, else the `<name>@<zone>` variant nearest to `near`.
    pub fn resolve(&self, name: &str, near: Vec3) -> Option<usize> {
        self.find(name).or_else(|| {
            let prefix = format!("{name}@");
            self.routes
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.deleted && r.name.starts_with(&prefix))
                .min_by(|a, b| a.1.distance_to(near).total_cmp(&b.1.distance_to(near)))
                .map(|(i, _)| i)
        })
    }

    /// Remembers route `i` before an edit (one undo step).
    fn snapshot(&mut self, i: usize) {
        if let Some(r) = self.routes.get(i) {
            self.undo.push((i, r.clone()));
            self.redo.clear();
        }
    }

    fn undo(&mut self) -> bool {
        let Some((i, r)) = self.undo.pop() else { return false };
        let now = std::mem::replace(&mut self.routes[i], r);
        self.redo.push((i, now));
        self.selected = Some(i);
        self.point = None;
        true
    }

    fn redo(&mut self) -> bool {
        let Some((i, r)) = self.redo.pop() else { return false };
        let now = std::mem::replace(&mut self.routes[i], r);
        self.undo.push((i, now));
        self.selected = Some(i);
        self.point = None;
        true
    }

    /// Adds a new, empty route (unique name) and selects it.
    fn create(&mut self, name: &str) -> Result<usize, String> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(format!("\"{name}\" is not a usable file name (letters, digits, _ - @ .)"));
        }
        if self.find(name).is_some() || self.routes.iter().any(|r| r.saved.as_ref().is_some_and(|(n, _)| n == name) && !r.deleted) {
            return Err(format!("a route named {name} exists"));
        }
        self.routes.push(Route { name: name.to_string(), points: Vec::new(), saved: None, deleted: false });
        let i = self.routes.len() - 1;
        self.undo.push((i, Route { name: name.to_string(), points: Vec::new(), saved: None, deleted: true }));
        self.redo.clear();
        self.selected = Some(i);
        self.point = None;
        Ok(i)
    }

    /// Writes every changed route. Returns the files written or removed.
    pub fn save(&mut self) -> anyhow::Result<Vec<PathBuf>> {
        let mut done = Vec::new();
        let mut folders = vec![self.dir.clone()];
        folders.extend(self.mirrors.iter().filter(|m| m.is_dir()).cloned());
        std::fs::create_dir_all(&self.dir)?;
        for r in self.routes.iter_mut().filter(|r| r.changed()) {
            for folder in &folders {
                let keep = |file: &Path| -> std::io::Result<()> {
                    let backup = file.with_extension("path.editor-orig");
                    if file.exists() && !backup.exists() {
                        std::fs::copy(file, &backup)?;
                    }
                    Ok(())
                };
                // the old file goes when the route is deleted or renamed
                if let Some((old, _)) = &r.saved
                    && (r.deleted || *old != r.name)
                {
                    let file = folder.join(format!("{old}.path"));
                    if file.exists() {
                        keep(&file)?;
                        std::fs::remove_file(&file)?;
                        done.push(file);
                    }
                }
                if !r.deleted {
                    let file = folder.join(format!("{}.path", r.name));
                    keep(&file)?;
                    let tmp = file.with_extension("path.editor-tmp");
                    std::fs::write(&tmp, write(&r.points))?;
                    std::fs::rename(&tmp, &file)?;
                    done.push(file);
                }
            }
            r.saved = (!r.deleted).then(|| (r.name.clone(), r.points.clone()));
        }
        Ok(done)
    }
}

/// The path folder for a spawn folder (see the module documentation); `mirrors` are the spawn mirrors.
pub fn path_dir(spawn_dir: &Path) -> PathBuf {
    if let Some(d) = crate::arg("--path-dir") {
        return PathBuf::from(d);
    }
    let server = spawn_dir.parent().and_then(Path::parent).map(|d| d.join("Path"));
    if let Some(d) = server.filter(|d| d.is_dir()) {
        return d;
    }
    spawn_dir.parent().map(|d| d.join("Path")).unwrap_or_else(|| PathBuf::from("Path"))
}

fn copy_lane_paths(dir: &Path) {
    let lane = Path::new(crate::worlds::LANE_DATA).join("Path");
    if dir.exists() || !lane.is_dir() || dir == lane {
        return;
    }
    let copied = std::fs::create_dir_all(dir).and_then(|_| {
        let mut n = 0;
        for e in std::fs::read_dir(&lane)?.flatten() {
            if e.path().extension().is_some_and(|x| x == "path") {
                std::fs::copy(e.path(), dir.join(e.file_name()))?;
                n += 1;
            }
        }
        Ok(n)
    });
    match copied {
        Ok(n) => info!("ai paths: copied {n} routes from {} to {}", lane.display(), dir.display()),
        Err(e) => warn!("ai paths: cannot copy the lane's routes to {}: {e}", dir.display()),
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(Update, (interact, keys, save, draw).chain().run_if(resource_exists::<AiPaths>))
        .add_systems(EguiPrimaryContextPass, (window, labels).after(crate::ui::panels).run_if(resource_exists::<AiPaths>));
}

fn setup(mut commands: Commands, mut editor: ResMut<Editor>) {
    let dir = path_dir(&editor.paths.spawn_dir);
    copy_lane_paths(&dir);
    let mirrors = editor
        .paths
        .mirrors
        .iter()
        .filter_map(|m| m.parent().and_then(Path::parent).map(|d| d.join("Path")))
        .filter(|d| d.is_dir() && *d != dir)
        .collect();
    let mut paths = AiPaths::load(&dir, mirrors);
    paths.open = editor.mode == Mode::Paths || crate::flag("--paths");
    // `--path <name>`: select that route at start
    paths.selected = crate::arg("--path").and_then(|n| paths.find(&n));
    if let Some(p) = paths.selected.and_then(|i| paths.routes[i].points.first()) {
        editor.requests.push(Request::Focus(p.pos));
    }
    commands.insert_resource(paths);
}

fn ground(heights: &Heights, query: &TerrainQuery, p: Vec3) -> f32 {
    heights.height_at(query, p.x, p.y).unwrap_or(p.z)
}

/// The routes drawn now: near the camera in Paths mode, plus the selected one and the selected NPC's.
fn shown(paths: &AiPaths, editor: &Editor, store: &SpawnStore, camera: Vec3) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    if editor.mode == Mode::Paths {
        out.extend(paths.routes.iter().enumerate().filter(|(_, r)| !r.deleted && r.distance_to(camera) < paths.radius).map(|(i, _)| i));
    }
    out.extend(paths.selected.filter(|&i| !paths.routes[i].deleted));
    if let Some(s) = editor.selected.and_then(|i| store.spawns.get(i)).filter(|s| !s.now.deleted)
        && let Some(i) = store.path_of(&s.now).and_then(|n| paths.resolve(n, s.now.pos))
    {
        out.push(i);
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[allow(clippy::too_many_arguments)]
fn interact(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Res<Cursor>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    window: Query<&Window, With<PrimaryWindow>>,
    cam: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    mut editor: ResMut<Editor>,
    store: Res<SpawnStore>,
    mut paths: ResMut<AiPaths>,
) {
    if editor.mode != Mode::Paths || editor.playing {
        paths.hover = None;
        paths.drag = None;
        return;
    }
    let (Ok(window), Ok((camera, ct))) = (window.single(), cam.single()) else { return };
    let mouse_pos = window.cursor_position();
    // hover: the nearest point within 12 px, else a segment within 6 px (the selected route wins ties)
    paths.hover = None;
    if let Some(m) = mouse_pos.filter(|_| !cursor.over_ui && paths.drag.is_none()) {
        let screen = |p: Vec3| camera.world_to_viewport(ct, cry_to_bevy(p)).ok();
        let mut best: Option<(f32, usize, Option<usize>)> = None;
        for i in shown(&paths, &editor, &store, bevy_to_cry(ct.translation())) {
            let bonus = if paths.selected == Some(i) { 3.0 } else { 0.0 };
            let pts: Vec<Option<Vec2>> = paths.routes[i].points.iter().map(|p| screen(p.pos + Vec3::Z * 0.3)).collect();
            for (k, s) in pts.iter().enumerate() {
                if let Some(s) = s {
                    let d = s.distance(m) - bonus;
                    if d < 12.0 && best.is_none_or(|b| d < b.0) {
                        best = Some((d, i, Some(k)));
                    }
                }
            }
            for (k, w) in pts.windows(2).enumerate() {
                let (Some(a), Some(b)) = (w[0], w[1]) else { continue };
                let ab = b - a;
                let t = if ab.length_squared() > 0.0 { ((m - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                let d = (a + ab * t).distance(m) + 6.0 - bonus;
                if d < 12.0 && best.is_none_or(|b| d < b.0) {
                    best = Some((d, i, None));
                    let _ = k;
                }
            }
        }
        paths.hover = best.map(|(_, i, k)| (i, k));
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if mouse.just_pressed(MouseButton::Left) && !cursor.over_ui {
        if shift {
            let (Some(i), Some(g)) = (paths.selected.filter(|&i| !paths.routes[i].deleted), cursor.ground) else {
                editor.status("paths: select a route (or make a new one) before adding points");
                return;
            };
            paths.snapshot(i);
            let at = paths.point.map_or(paths.routes[i].points.len(), |k| (k + 1).min(paths.routes[i].points.len()));
            paths.routes[i].points.insert(at, Pt::at(g));
            paths.point = Some(at);
            editor.status(format!("paths: point {} added to {}", at + 1, paths.routes[i].name));
            return;
        }
        match paths.hover {
            Some((i, Some(k))) => {
                paths.selected = Some(i);
                paths.point = Some(k);
                let p = paths.routes[i].points[k].pos;
                paths.drag = Some(Drag { route: i, point: k, above: p.z - ground(&heights, &query, p), press: mouse_pos.unwrap_or_default(), started: false });
            }
            Some((i, None)) => {
                paths.selected = Some(i);
                paths.point = None;
            }
            None => paths.point = None,
        }
    }
    if !mouse.pressed(MouseButton::Left) {
        paths.drag = None;
        return;
    }
    let (Some(g), Some(m)) = (cursor.ground, mouse_pos) else { return };
    let snap = paths.snap;
    let Some(d) = paths.drag.as_mut() else { return };
    if !d.started && d.press.distance(m) < 4.0 {
        return;
    }
    let (i, k, above, first) = (d.route, d.point, d.above, !d.started);
    d.started = true;
    if first {
        paths.snapshot(i);
    }
    let Some(p) = paths.routes.get_mut(i).and_then(|r| r.points.get_mut(k)) else { return };
    p.pos = if snap { Vec3::new(g.x, g.y, ground(&heights, &query, g) + above.clamp(-2.0, 2.0)) } else { Vec3::new(g.x, g.y, p.pos.z) };
}

fn keys(keys: Res<ButtonInput<KeyCode>>, egui: Res<EguiWantsInput>, mut editor: ResMut<Editor>, mut paths: ResMut<AiPaths>, time: Res<Time>) {
    if let Some(w) = paths.walk.as_mut() {
        *w += WALK_SPEED * time.delta_secs();
    }
    if keys.just_pressed(KeyCode::Digit5) && !egui.wants_any_keyboard_input() {
        editor.mode = Mode::Paths;
        paths.open = true;
    }
    if editor.mode != Mode::Paths || editor.playing || egui.wants_any_keyboard_input() {
        return;
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if ctrl && keys.just_pressed(KeyCode::KeyZ) && paths.undo() {
        editor.status("paths: undo");
    }
    if ctrl && keys.just_pressed(KeyCode::KeyY) && paths.redo() {
        editor.status("paths: redo");
    }
    if keys.just_pressed(KeyCode::Escape) {
        paths.point = None;
    }
    if (keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace))
        && let (Some(i), Some(k)) = (paths.selected, paths.point)
        && k < paths.routes[i].points.len()
    {
        paths.snapshot(i);
        paths.routes[i].points.remove(k);
        paths.point = k.checked_sub(1).or((!paths.routes[i].points.is_empty()).then_some(0));
        editor.status(format!("paths: point {} removed", k + 1));
    }
    if keys.just_pressed(KeyCode::KeyF)
        && !ctrl
        && let Some(i) = paths.selected
        && let Some(p) = paths.point.and_then(|k| paths.routes[i].points.get(k)).or(paths.routes[i].points.first())
    {
        editor.requests.push(Request::Focus(p.pos));
    }
}

fn save(mut editor: ResMut<Editor>, mut paths: ResMut<AiPaths>, mut game: ResMut<GameData>) {
    let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::SavePaths)) else { return };
    editor.requests.remove(i);
    let n = paths.routes.iter().filter(|r| r.changed()).count();
    let mut said = Vec::new();
    if n > 0 {
        said.push(match paths.save() {
            Ok(files) => format!("{n} route(s) saved ({} file(s) in {})", files.len(), paths.dir.display()),
            Err(e) => format!("route save failed: {e:#}"),
        });
    }
    let c = paths.commands_changed();
    if c > 0
        && let Some(now) = paths.commands.clone()
    {
        said.push(match game.save_path_commands(&paths.commands_saved, &now) {
            Ok(rows) => {
                // read back: the new rows get their ids
                let fresh = game.path_commands();
                paths.commands_saved = fresh.clone();
                paths.commands = Some(fresh);
                format!("{rows} AI command row(s) written to {}", game.path)
            }
            Err(e) => format!("{c} AI command change(s) not written: {e:#}"),
        });
    }
    if !said.is_empty() {
        editor.status(format!("paths: {}; restart the server to load them", said.join(", ")));
    }
}

fn draw(editor: Res<Editor>, store: Res<SpawnStore>, paths: Res<AiPaths>, cam: Query<&GlobalTransform, With<EditorCamera>>, mut gizmos: Gizmos) {
    if editor.playing {
        return;
    }
    let Ok(ct) = cam.single() else { return };
    let lift = |p: Vec3| cry_to_bevy(p + Vec3::Z * 0.3);
    for i in shown(&paths, &editor, &store, bevy_to_cry(ct.translation())) {
        let r = &paths.routes[i];
        let selected = paths.selected == Some(i);
        let hovered = paths.hover.is_some_and(|h| h.0 == i);
        let color = if selected {
            Color::srgb(1.0, 0.8, 0.15)
        } else if hovered {
            Color::WHITE
        } else {
            Color::srgba(0.3, 0.85, 1.0, 0.85)
        };
        for w in r.points.windows(2) {
            let (a, b) = (lift(w[0].pos), lift(w[1].pos));
            gizmos.line(a, b, color);
            let len = a.distance(b);
            if len > 3.0 {
                let mid = (a + b) / 2.0;
                let dir = (b - a) / len;
                gizmos.arrow(mid - dir * 0.8, mid + dir * 0.8, color);
            }
        }
        let n = r.points.len();
        for (k, p) in r.points.iter().enumerate() {
            let c = if k == 0 {
                Color::srgb(0.3, 1.0, 0.35)
            } else if k + 1 == n {
                Color::srgb(1.0, 0.35, 0.3)
            } else {
                color
            };
            let hot = selected && paths.point == Some(k) || paths.hover == Some((i, Some(k)));
            let at = lift(p.pos);
            gizmos.sphere(Isometry3d::from_translation(at), if hot { 0.6 } else { 0.3 }, c);
            if !p.action.is_empty() {
                gizmos.line(at, at + Vec3::Y * 1.5, Color::srgb(1.0, 0.4, 1.0));
            }
        }
        // the NPCs that walk it
        for s in store.spawns.iter().filter(|s| s.kind == Kind::Npc && !s.now.deleted) {
            if let Some(first) = r.points.first()
                && store.path_of(&s.now).is_some_and(|n| paths.resolve(n, s.now.pos) == Some(i))
                && s.now.pos.distance(first.pos) < 2000.0
            {
                gizmos.line(cry_to_bevy(s.now.pos + Vec3::Z * 1.0), lift(first.pos), Color::srgba(1.0, 0.4, 1.0, 0.7));
            }
        }
        if selected
            && let Some(p) = paths.walk.and_then(|d| r.along(d))
        {
            gizmos.sphere(Isometry3d::from_translation(cry_to_bevy(p + Vec3::Z * 0.9)), 0.9, Color::srgb(1.0, 0.4, 1.0));
        }
    }
}

/// Route names in the view and point numbers of the selected route.
fn labels(
    mut contexts: EguiContexts,
    editor: Res<Editor>,
    store: Res<SpawnStore>,
    paths: Res<AiPaths>,
    area: Res<crate::ui::UiArea>,
    cam: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
) -> Result {
    if editor.playing {
        return Ok(());
    }
    let Ok((camera, ct)) = cam.single() else { return Ok(()) };
    let ctx = contexts.ctx_mut()?;
    let mut painter = ctx.layer_painter(egui::LayerId::background());
    if let Some(view) = area.view {
        painter.set_clip_rect(view);
    }
    let screen = |p: Vec3| camera.world_to_viewport(ct, cry_to_bevy(p)).ok().map(|s| egui::pos2(s.x, s.y));
    let text = |at: egui::Pos2, s: &str, color: egui::Color32, size: f32| {
        painter.text(at + egui::vec2(1.0, 1.0), egui::Align2::CENTER_BOTTOM, s, egui::FontId::proportional(size), egui::Color32::BLACK);
        painter.text(at, egui::Align2::CENTER_BOTTOM, s, egui::FontId::proportional(size), color);
    };
    let cam_pos = bevy_to_cry(ct.translation());
    for i in shown(&paths, &editor, &store, cam_pos) {
        let r = &paths.routes[i];
        let selected = paths.selected == Some(i);
        let Some(first) = r.points.first() else { continue };
        if !selected && first.pos.distance(cam_pos) > 250.0 && paths.hover.is_none_or(|h| h.0 != i) {
            continue;
        }
        if let Some(s) = screen(first.pos + Vec3::Z * 2.2) {
            let color = if selected { egui::Color32::from_rgb(255, 205, 40) } else { egui::Color32::from_rgb(120, 220, 255) };
            text(s, &format!("⛳ {}", r.name), color, 14.0);
        }
        if selected {
            for (k, p) in r.points.iter().enumerate() {
                if p.pos.distance(cam_pos) > 200.0 {
                    continue;
                }
                if let Some(s) = screen(p.pos + Vec3::Z * 1.0) {
                    let label = if p.action.is_empty() { format!("{}", k + 1) } else { format!("{} {} {}", k + 1, p.action, p.param) };
                    text(s, &label, egui::Color32::WHITE, 12.0);
                }
            }
        }
    }
    Ok(())
}

/// The AI path part of the inspector for the selected NPC spawn.
pub fn spawn_section(ui: &mut egui::Ui, editor: &mut Editor, store: &mut SpawnStore, paths: &mut AiPaths, i: usize) {
    let mut p = store.spawns[i].now;
    let current = store.path_of(&p).map(str::to_string);
    ui.horizontal_wrapped(|ui| {
        ui.label("AI path");
        match &current {
            Some(name) => {
                let found = paths.resolve(name, p.pos);
                let shown = match found {
                    Some(r) if paths.routes[r].name != *name => format!("{name} → {}", paths.routes[r].name),
                    Some(_) => name.clone(),
                    None => format!("{name} (no such file)"),
                };
                if ui.link(shown).on_hover_text("Edit this route (Paths mode)").clicked()
                    && let Some(r) = found
                {
                    paths.selected = Some(r);
                    paths.point = None;
                    paths.open = true;
                    editor.mode = Mode::Paths;
                }
                if ui.small_button("✖").on_hover_text("This NPC stops following a route").clicked() {
                    p.path = 0;
                    store.apply(vec![(i, p)]);
                }
            }
            None => {
                ui.weak("none");
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if let Some(r) = paths.selected.filter(|&r| !paths.routes[r].deleted)
            && current.as_deref() != Some(paths.routes[r].name.as_str())
            && ui.button(format!("Follow {}", paths.routes[r].name)).on_hover_text("Set this spawn's FollowPath to the selected route").clicked()
        {
            p.path = store.path_id(&paths.routes[r].name.clone());
            store.apply(vec![(i, p)]);
        }
        if ui.button("New route from here").on_hover_text("A new route starting at this NPC, which then follows it").clicked() {
            let unit = p.unit_id;
            let base = format!("editor_{unit}");
            let name = (0..1000).map(|n| format!("{base}_{n:02}")).find(|n| paths.find(n).is_none() && !paths.dir.join(format!("{n}.path")).exists()).unwrap_or(base);
            match paths.create(&name) {
                Ok(r) => {
                    paths.routes[r].points.push(Pt::at(p.pos));
                    paths.point = Some(0);
                    p.path = store.path_id(&name);
                    store.apply(vec![(i, p)]);
                    paths.open = true;
                    editor.mode = Mode::Paths;
                    editor.status(format!("paths: new route {name}; Shift+click the ground to add points"));
                }
                Err(e) => editor.status(e),
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
/// The FollowPath AI commands `rows` of route `name`: re-point one to another route, remove one, add one to a set.
fn command_rows(ui: &mut egui::Ui, paths: &mut AiPaths, rows: &[usize], name: &str) {
    let mut names: Vec<String> = paths.routes.iter().filter(|r| !r.deleted).map(|r| r.name.clone()).collect();
    names.sort();
    let mut remove = None;
    if let Some(commands) = &mut paths.commands {
        for &k in rows {
            let c = &mut commands[k];
            ui.horizontal(|ui| {
                let id = if c.id == 0 { "new".to_string() } else { format!("#{}", c.id) };
                ui.label(format!("AI command set {} {} ({id})", c.set, c.set_name)).on_hover_text("ai_commands row, cmd_id 2: follow the route");
                egui::ComboBox::from_id_salt(("command route", k)).width(150.0).selected_text(c.route.as_str()).show_ui(ui, |ui| {
                    for n in &names {
                        ui.selectable_value(&mut c.route, n.clone(), n.as_str());
                    }
                });
                if ui.small_button("✖").on_hover_text("Remove this command from its set").clicked() {
                    remove = Some(k);
                }
            });
        }
    }
    if let Some(k) = remove
        && let Some(c) = &mut paths.commands
    {
        c.remove(k);
    }
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut paths.add_set).hint_text("command set id").desired_width(110.0));
        let id = paths.add_set.trim().parse::<u32>().ok();
        let known = id.and_then(|id| paths.sets.iter().flatten().find(|s| s.0 == id).cloned());
        let tip = match (&known, id) {
            (Some(s), _) => format!("Add \"follow {name}\" to set {} {}", s.0, s.1),
            (None, Some(id)) => format!("No AI command set {id}"),
            (None, None) => "The id of an AI command set (ai_command_sets)".to_string(),
        };
        if ui.add_enabled(known.is_some(), egui::Button::new("Add follow command")).on_hover_text(tip).clicked()
            && let Some((set, set_name)) = known
            && let Some(c) = &mut paths.commands
        {
            c.push(PathCommand { id: 0, set, set_name, param1: 1, route: name.to_string() });
            paths.add_set.clear();
        }
    });
}

fn window(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut store: ResMut<SpawnStore>,
    mut paths: ResMut<AiPaths>,
    game: Res<GameData>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    cam: Query<&GlobalTransform, With<EditorCamera>>,
) -> Result {
    if editor.playing || editor.mode != Mode::Paths {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    let cam_pos = cam.single().map(|t| bevy_to_cry(t.translation())).unwrap_or_default();
    if paths.sets.is_none() {
        paths.sets = Some(game.command_sets());
    }
    if paths.commands.is_none() {
        let read = game.path_commands();
        paths.commands_saved = read.clone();
        paths.commands = Some(read);
    }
    let mut open = paths.open;
    egui::Window::new("AI paths").open(&mut open).default_pos([330.0, 60.0]).default_size([430.0, 640.0]).resizable(true).show(&ctx, |ui| {
        let paths = &mut *paths;
        ui.small(format!("{} routes in {}", paths.routes.iter().filter(|r| !r.deleted).count(), paths.dir.display()));
        ui.horizontal(|ui| {
            let n = paths.changed_count();
            if ui.add_enabled(n > 0, egui::Button::new(format!("Save {n} route(s)"))).clicked() {
                editor.requests.push(Request::SavePaths);
            }
            ui.checkbox(&mut paths.snap, "drag on the ground");
            ui.add(egui::DragValue::new(&mut paths.radius).range(50.0..=5000.0).speed(10.0).suffix(" m")).on_hover_text("Routes within this distance of the camera are drawn");
        });
        ui.small("Click a point to select, drag to move · Shift+click the ground: add a point after the selected one · Del: remove · Ctrl+Z/Y · F: focus");
        ui.separator();

        // new route
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut paths.new_name).hint_text("new route name").desired_width(170.0));
            if ui.button("New route").clicked() {
                let name = paths.new_name.clone();
                match paths.create(&name) {
                    Ok(_) => {
                        paths.new_name.clear();
                        editor.status(format!("paths: new route {name}; Shift+click the ground to add points"));
                    }
                    Err(e) => editor.status(e),
                }
            }
        });

        // the list
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.add(egui::TextEdit::singleline(&mut paths.search).desired_width(160.0));
            ui.checkbox(&mut paths.near_only, "near the camera");
        });
        let needle = paths.search.trim().to_lowercase();
        let mut list: Vec<(usize, f32)> = paths
            .routes
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.deleted && (needle.is_empty() || r.name.to_lowercase().contains(&needle)))
            .map(|(i, r)| (i, r.distance_to(cam_pos)))
            .filter(|(_, d)| !paths.near_only || !needle.is_empty() || *d < paths.radius * 3.0)
            .collect();
        list.sort_by(|a, b| a.1.total_cmp(&b.1));
        let routes_named: Vec<String> = paths.commands.iter().flatten().map(|c| c.route.clone()).collect();
        let users = |name: &str| -> (usize, usize) {
            let names = |n: &str| n == name || name.starts_with(&format!("{n}@"));
            let spawns = store.spawns.iter().filter(|s| !s.now.deleted && store.path_of(&s.now).is_some_and(names)).count();
            (spawns, routes_named.iter().filter(|n| names(n)).count())
        };
        egui::ScrollArea::vertical().id_salt("routes").max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
            if list.is_empty() {
                ui.weak(if paths.near_only { "No route near the camera (untick \"near the camera\" or search)." } else { "No route." });
            }
            for (i, d) in list.iter().take(400) {
                let r = &paths.routes[*i];
                let (s, c) = users(&r.name);
                let mark = if r.changed() { "● " } else { "" };
                let used = match (s, c) {
                    (0, 0) => String::new(),
                    (s, 0) => format!(" · {s} NPC"),
                    (0, c) => format!(" · {c} AI cmd"),
                    (s, c) => format!(" · {s} NPC, {c} AI cmd"),
                };
                let label = format!("{mark}{}  ({} pts{used}) {:.0} m", r.name, r.points.len(), d);
                if ui.selectable_label(paths.selected == Some(*i), label).clicked() {
                    paths.selected = Some(*i);
                    paths.point = None;
                    if let Some(p) = r.points.first() {
                        editor.requests.push(Request::Focus(p.pos));
                    }
                }
            }
        });
        ui.separator();

        // the selected route
        let Some(i) = paths.selected.filter(|&i| i < paths.routes.len() && !paths.routes[i].deleted) else {
            ui.weak("Select a route in the list or the view.");
            return;
        };
        if paths.rename_for != Some(i) {
            paths.rename_for = Some(i);
            paths.rename = paths.routes[i].name.clone();
        }
        ui.horizontal(|ui| {
            ui.label("Name");
            let r = ui.add(egui::TextEdit::singleline(&mut paths.rename).desired_width(200.0));
            if r.lost_focus() && paths.rename != paths.routes[i].name {
                let new = paths.rename.trim().to_string();
                if !valid_name(&new) || paths.find(&new).is_some() {
                    editor.status(format!("paths: cannot rename to \"{new}\""));
                    paths.rename = paths.routes[i].name.clone();
                } else {
                    let old = paths.routes[i].name.clone();
                    paths.snapshot(i);
                    paths.routes[i].name = new.clone();
                    // the spawns that named it follow the new name
                    let new_id = store.path_id(&new);
                    let changes: Vec<(usize, crate::spawns::Placement)> = (0..store.spawns.len())
                        .filter(|&s| store.path_of(&store.spawns[s].now) == Some(old.as_str()))
                        .map(|s| (s, crate::spawns::Placement { path: new_id, ..store.spawns[s].now }))
                        .collect();
                    let n = changes.len();
                    store.apply(changes);
                    let named: Vec<usize> = paths.commands.iter().flatten().enumerate().filter(|(_, c)| c.route == old).map(|(k, _)| k).collect();
                    let cmds = if named.is_empty() {
                        String::new()
                    } else if paths.rename_commands
                        && let Some(c) = &mut paths.commands
                    {
                        for &k in &named {
                            c[k].route = new.clone();
                        }
                        format!(", {} AI command(s) re-pointed (save to write them)", named.len())
                    } else {
                        format!(", {} AI command(s) in the database still name {old}", named.len())
                    };
                    editor.status(format!("paths: {old} renamed to {new}; {n} spawn(s) updated{cmds}"));
                }
            }
        });
        let named = paths.commands.iter().flatten().filter(|c| c.route == paths.routes[i].name).count();
        if named > 0 {
            ui.checkbox(&mut paths.rename_commands, format!("renaming also updates the {named} AI command(s) that name it"));
        }
        let r = &paths.routes[i];
        ui.small(format!("{} points · {:.0} m{}", r.points.len(), r.length(), if r.saved.is_none() { " · new" } else { "" }));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Focus").clicked()
                && let Some(p) = paths.routes[i].points.first()
            {
                editor.requests.push(Request::Focus(p.pos));
            }
            let walking = paths.walk.is_some();
            if ui.button(if walking { "■ Stop walk" } else { "▶ Walk" }).on_hover_text("A marker walks the route at 3 m/s").clicked() {
                paths.walk = if walking { None } else { Some(0.0) };
            }
            if ui.button("Reverse").clicked() {
                paths.snapshot(i);
                paths.routes[i].points.reverse();
            }
            if ui.button("Close loop").on_hover_text("Add the first point again at the end").clicked()
                && let Some(p) = paths.routes[i].points.first().cloned()
            {
                paths.snapshot(i);
                paths.routes[i].points.push(Pt::at(p.pos));
            }
            if ui.button("Snap to ground").clicked() {
                paths.snapshot(i);
                for p in &mut paths.routes[i].points {
                    p.pos.z = ground(&heights, &query, p.pos);
                }
            }
            if ui.button("Duplicate").clicked() {
                let base = paths.routes[i].name.clone();
                let name = (2..1000).map(|n| format!("{base}_copy{n}")).find(|n| paths.find(n).is_none()).unwrap_or(base.clone());
                let points = paths.routes[i].points.clone();
                if paths.create(&name).is_ok()
                    && let Some(j) = paths.selected
                {
                    paths.routes[j].points = points;
                }
            }
            if ui.button("Delete route").clicked() {
                paths.snapshot(i);
                paths.routes[i].deleted = true;
                paths.selected = None;
                editor.status("paths: route deleted (its file goes when you save)");
            }
        });
        let Some(i) = paths.selected else { return };
        let name = paths.routes[i].name.clone();

        // who uses it
        ui.collapsing("Used by", |ui| {
            let spawns: Vec<usize> = (0..store.spawns.len())
                .filter(|&s| !store.spawns[s].now.deleted && store.path_of(&store.spawns[s].now).is_some_and(|n| paths.resolve(n, store.spawns[s].now.pos) == Some(i)))
                .collect();
            for s in &spawns {
                let sp = &store.spawns[*s];
                let label = format!("NPC {} {}", sp.now.unit_id, game.name(crate::db::Table::Npcs, sp.now.unit_id).unwrap_or_default());
                if ui.link(label).clicked() {
                    editor.selected = Some(*s);
                    editor.requests.push(Request::Focus(sp.now.pos));
                }
            }
            let sets = paths.commands_of(&name);
            command_rows(ui, paths, &sets, &name);
            if spawns.is_empty() && sets.is_empty() {
                ui.weak("Nothing names this route yet.");
            }
            if let Some(s) = editor.selected.filter(|&s| s < store.spawns.len() && store.spawns[s].kind == Kind::Npc && !store.spawns[s].now.deleted) {
                let mut p = store.spawns[s].now;
                if store.path_of(&p) != Some(name.as_str())
                    && ui.button(format!("Selected NPC {} follows {name}", p.unit_id)).clicked()
                {
                    p.path = store.path_id(&name);
                    store.apply(vec![(s, p)]);
                    editor.status(format!("NPC {} now follows {name} (save spawns, restart the server)", p.unit_id));
                }
            } else {
                ui.weak("Select an NPC (Select mode) to make it follow this route.");
            }
        });

        // the points
        ui.label("Points");
        let mut edit: Option<Box<dyn FnOnce(&mut Vec<Pt>)>> = None;
        let mut select = None;
        let n = paths.routes[i].points.len();
        egui::ScrollArea::vertical().id_salt("points").auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("route points").num_columns(8).striped(true).show(ui, |ui| {
                ui.strong("#");
                ui.strong("action");
                ui.strong("param");
                ui.strong("X");
                ui.strong("Y");
                ui.strong("Z");
                ui.label("");
                ui.end_row();
                for k in 0..n {
                    let mut p = paths.routes[i].points[k].clone();
                    if ui.selectable_label(paths.point == Some(k), format!("{}", k + 1)).clicked() {
                        select = Some(k);
                    }
                    egui::ComboBox::from_id_salt(("action", k)).width(90.0).selected_text(if p.action.is_empty() { "–" } else { p.action.as_str() }).show_ui(ui, |ui| {
                        for a in ACTIONS {
                            ui.selectable_value(&mut p.action, a.to_string(), if a.is_empty() { "–" } else { a });
                        }
                    });
                    ui.add(egui::TextEdit::singleline(&mut p.param).desired_width(48.0));
                    ui.add(egui::DragValue::new(&mut p.pos.x).speed(0.1).max_decimals(2));
                    ui.add(egui::DragValue::new(&mut p.pos.y).speed(0.1).max_decimals(2));
                    ui.add(egui::DragValue::new(&mut p.pos.z).speed(0.05).max_decimals(2));
                    ui.horizontal(|ui| {
                        if ui.small_button("+").on_hover_text("Insert a point after this one").clicked() {
                            edit = Some(Box::new(move |pts: &mut Vec<Pt>| {
                                let next = pts.get(k + 1).map_or(pts[k].pos + Vec3::new(3.0, 0.0, 0.0), |q| (pts[k].pos + q.pos) / 2.0);
                                pts.insert(k + 1, Pt::at(next));
                            }));
                        }
                        if k > 0 && ui.small_button("⬆").clicked() {
                            edit = Some(Box::new(move |pts: &mut Vec<Pt>| pts.swap(k, k - 1)));
                        }
                        if k + 1 < n && ui.small_button("⬇").clicked() {
                            edit = Some(Box::new(move |pts: &mut Vec<Pt>| pts.swap(k, k + 1)));
                        }
                        if ui.small_button("✖").clicked() {
                            edit = Some(Box::new(move |pts: &mut Vec<Pt>| {
                                pts.remove(k);
                            }));
                        }
                    });
                    ui.end_row();
                    if p != paths.routes[i].points[k] {
                        // one undo step per field edit burst: snapshot when the point was not the last edited
                        if paths.undo.last().is_none_or(|(u, r)| *u != i || r.points.len() != n) {
                            paths.snapshot(i);
                        }
                        paths.routes[i].points[k] = p;
                    }
                }
            });
            if n == 0 {
                ui.weak("No points: Shift+click the ground in the view.");
            }
        });
        if let Some(k) = select {
            paths.point = Some(k);
            editor.requests.push(Request::Focus(paths.routes[i].points[k].pos));
        }
        if let Some(f) = edit {
            paths.snapshot(i);
            f(&mut paths.routes[i].points);
            paths.point = None;
        }
    });
    paths.open = open;
    if !open {
        paths.open = true;
        editor.mode = Mode::Select;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_round_trip() {
        let text = "|17741.116|8397.060|276.379|\r\n|17743.887|8394.471|274.595|\r\nSpeed|1|2|3|5\r\nbad line\r\n";
        let pts = parse(text);
        assert_eq!(pts.len(), 3);
        assert_eq!((pts[2].action.as_str(), pts[2].pos, pts[2].param.as_str()), ("Speed", Vec3::new(1.0, 2.0, 3.0), "5"));
        // an edited axis is written fresh, the others as they were
        let mut moved = pts[0].clone();
        moved.pos.z = 280.0;
        assert_eq!(write(&[moved]), "|17741.116|8397.060|280.000|\r\n");
        assert_eq!(num(1.5), "1.500");
        assert_eq!(num(1234.5), "1234.500");
        assert_eq!(write(&pts[..2]), "|17741.116|8397.060|276.379|\r\n|17743.887|8394.471|274.595|\r\n");
        assert_eq!(parse(&write(&pts)), pts);
    }

    #[test]
    fn command_edits_count_as_unsaved() {
        let mut p = AiPaths::empty(PathBuf::from("unused"), Vec::new());
        let c = |id, route: &str| PathCommand { id, set: 76, set_name: "kron".into(), param1: 1, route: route.into() };
        p.commands_saved = vec![c(1058, "a"), c(1061, "a")];
        p.commands = Some(p.commands_saved.clone());
        assert_eq!(p.changed_count(), 0);
        assert_eq!(p.commands_of("a@zone"), vec![0, 1]);
        let now = p.commands.as_mut().unwrap();
        now[0].route = "b".into();
        now.remove(1);
        now.push(c(0, "a"));
        assert_eq!(p.changed_count(), 3);
    }

    #[test]
    fn create_save_rename_delete() {
        let dir = std::env::temp_dir().join(format!("aa-editor-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.path"), "|1.000|2.000|3.000|\r\n").unwrap();
        let mut p = AiPaths::load(&dir, Vec::new());
        assert_eq!(p.routes.len(), 1);
        assert_eq!(p.changed_count(), 0);
        let i = p.create("walk_a").unwrap();
        assert!(p.create("walk_a").is_err());
        assert!(p.create("../x").is_err());
        p.routes[i].points = vec![Pt::at(Vec3::new(10.0, 20.0, 30.0)), Pt::at(Vec3::new(11.0, 20.0, 30.0))];
        p.routes[0].name = "renamed".into();
        assert_eq!(p.changed_count(), 2);
        p.save().unwrap();
        assert_eq!(p.changed_count(), 0);
        assert_eq!(std::fs::read_to_string(dir.join("walk_a.path")).unwrap(), "|10.000|20.000|30.000|\r\n|11.000|20.000|30.000|\r\n");
        assert!(dir.join("renamed.path").exists() && !dir.join("old.path").exists() && dir.join("old.path.editor-orig").exists());
        p.routes[i].deleted = true;
        p.save().unwrap();
        assert!(!dir.join("walk_a.path").exists());
        // undo restores the route as it was before the edit
        p.snapshot(0);
        p.routes[0].points.clear();
        assert!(p.undo());
        assert_eq!(p.routes[0].points.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn variants_resolve_to_the_nearest() {
        let mut p = AiPaths::empty(PathBuf::new(), Vec::new());
        for (name, x) in [("farm@1", 0.0), ("farm@2", 1000.0)] {
            p.routes.push(Route { name: name.into(), points: vec![Pt::at(Vec3::new(x, 0.0, 0.0))], saved: None, deleted: false });
        }
        assert_eq!(p.resolve("farm", Vec3::new(900.0, 0.0, 0.0)), Some(1));
        assert_eq!(p.resolve("farm@1", Vec3::new(900.0, 0.0, 0.0)), Some(0));
        assert_eq!(p.resolve("other", Vec3::ZERO), None);
        let r = &p.routes[0];
        assert_eq!(r.along(5.0), Some(Vec3::ZERO));
    }

    #[test]
    fn lane_routes_parse() {
        let dir = Path::new(crate::worlds::LANE_DATA).join("Path");
        if !dir.is_dir() {
            return;
        }
        let p = AiPaths::load(&dir, Vec::new());
        assert!(p.routes.len() > 300);
        let empty: Vec<&str> = p.routes.iter().filter(|r| r.points.is_empty()).map(|r| r.name.as_str()).collect();
        assert!(empty.len() < 5, "routes without points: {empty:?}");
        // every route written back is the same file (bar files with lines the server skips)
        let mut same = 0;
        for r in &p.routes {
            assert_eq!(parse(&write(&r.points)), r.points);
            let text = std::fs::read_to_string(dir.join(format!("{}.path", r.name))).unwrap_or_default();
            if write(&r.points) == text.trim_start_matches('\u{feff}') {
                same += 1;
            }
        }
        assert!(same * 10 >= p.routes.len() * 9, "{same} of {} routes write back byte for byte", p.routes.len());
    }
}
