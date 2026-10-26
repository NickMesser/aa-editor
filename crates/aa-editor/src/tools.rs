//! The cursor and the spawn tools: select (click), move (drag the selection over the ground), turn (`[` `]`, Shift +
//! wheel), delete, duplicate, place a template from the browser, undo / redo, save.

use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;

use crate::camera::EditorCamera;
use crate::spawns::{Kind, Placement, SpawnStore};
use crate::terrain_edit::Heights;
use crate::views::SpawnView;
use crate::{Editor, Mode, Request};

/// Where the mouse points.
#[derive(Resource, Default)]
pub struct Cursor {
    /// Cry space ray.
    pub ray: Option<(Vec3, Vec3)>,
    /// Terrain point under the mouse, cry space.
    pub ground: Option<Vec3>,
    /// The spawn under the mouse.
    pub hover: Option<usize>,
    pub over_ui: bool,
}

/// An ongoing drag of the selection.
#[derive(Resource, Default)]
struct Drag {
    /// Selection position minus the ground point grabbed.
    offset: Option<Vec3>,
    /// Height above the terrain the selection keeps.
    above: f32,
    /// Window position of the press: the drag starts once the mouse moved away from it.
    press: Option<Vec2>,
    started: bool,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Cursor>()
        .init_resource::<Drag>()
        .add_systems(Update, (update_cursor, (click, drag, keys, draw_selection)).chain());
}

pub fn update_cursor(
    mut cursor: ResMut<Cursor>,
    window: Query<&Window, With<PrimaryWindow>>,
    cam: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    area: Res<crate::ui::UiArea>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    store: Res<SpawnStore>,
    views: Query<(&SpawnView, &GlobalTransform)>,
) {
    cursor.ray = None;
    cursor.ground = None;
    cursor.hover = None;
    let (Ok(window), Ok((camera, cam_t))) = (window.single(), cam.single()) else { return };
    let Some(pos) = window.cursor_position() else { return };
    cursor.over_ui = area.contains(pos * window.scale_factor());
    let Ok(ray) = camera.viewport_to_world(cam_t, pos) else { return };
    let origin = bevy_to_cry(ray.origin);
    let dir = bevy_to_cry(*ray.direction).normalize();
    cursor.ray = Some((origin, dir));
    cursor.ground = heights.raycast(&query, origin, dir, 4000.0);
    let ground_t = cursor.ground.map_or(f32::MAX, |g| g.distance(origin));
    // spawns: a sphere around each marker (scaled with the model's bounds)
    let mut best = (f32::MAX, None);
    for (view, t) in &views {
        let Some(s) = store.spawns.get(view.index) else { continue };
        let center = bevy_to_cry(t.translation()) + Vec3::Z * view.radius.min(3.0) * 0.6;
        let r = view.radius.clamp(0.8, 12.0);
        let oc = center - origin;
        let along = oc.dot(dir);
        if along < 0.0 {
            continue;
        }
        let d2 = oc.length_squared() - along * along;
        if d2 > r * r || along > ground_t + r {
            continue;
        }
        if along < best.0 && !s.now.deleted {
            best = (along, Some(view.index));
        }
    }
    cursor.hover = best.1;
}

fn ground_or(heights: &Heights, query: &TerrainQuery, p: Vec3) -> f32 {
    heights.height_at(query, p.x, p.y).unwrap_or(p.z)
}

#[allow(clippy::too_many_arguments)]
fn click(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Res<Cursor>,
    window: Query<&Window, With<PrimaryWindow>>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    mut editor: ResMut<Editor>,
    mut store: ResMut<SpawnStore>,
    mut drag: ResMut<Drag>,
    cam: Query<&GlobalTransform, With<EditorCamera>>,
) {
    if editor.playing || cursor.over_ui || !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    match editor.mode {
        Mode::Select => {
            editor.selected = cursor.hover;
            *drag = Drag::default();
            if let (Some(i), Some(g)) = (cursor.hover, cursor.ground) {
                let pos = store.spawns[i].now.pos;
                drag.offset = Some(pos - g);
                drag.above = pos.z - ground_or(&heights, &query, pos);
                drag.press = window.single().ok().and_then(Window::cursor_position);
            }
        }
        Mode::Place => {
            let (Some((kind, id)), Some(g)) = (editor.place, cursor.ground) else {
                if editor.place.is_none() {
                    editor.status("pick an NPC or doodad in the browser first");
                }
                return;
            };
            // face the camera
            let yaw = cam.single().map_or(0.0, |t| {
                let to_cam = bevy_to_cry(t.translation()) - g;
                (-to_cam.x).atan2(to_cam.y).to_degrees()
            });
            let index = store.add(kind, Placement::new(id, g, (yaw * 100.0).round() / 100.0));
            editor.selected = Some(index);
            if !keys.pressed(KeyCode::ShiftLeft) {
                editor.mode = Mode::Select;
            }
            editor.status(format!("placed {} {id} at {:.1}, {:.1}, {:.1} (Shift-click keeps placing)", kind.label(), g.x, g.y, g.z));
        }
        Mode::Terrain | Mode::Objects | Mode::Paths | Mode::Housing => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn drag(
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Res<Cursor>,
    editor: Res<Editor>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut store: ResMut<SpawnStore>,
    mut state: ResMut<Drag>,
) {
    if !mouse.pressed(MouseButton::Left) || editor.mode != Mode::Select || editor.playing {
        *state = Drag::default();
        return;
    }
    let (Some(i), Some(offset), Some(g)) = (editor.selected, state.offset, cursor.ground) else { return };
    if !state.started {
        let now = window.single().ok().and_then(Window::cursor_position);
        match (state.press, now) {
            (Some(a), Some(b)) if a.distance(b) >= 4.0 => {}
            _ => return,
        }
    }
    let mut p = store.spawns[i].now;
    let mut to = g + offset;
    to.z = if editor.snap_to_ground { ground_or(&heights, &query, to) + state.above } else { p.pos.z };
    if to.distance(p.pos) < 0.01 {
        return;
    }
    p.pos = to;
    if state.started {
        store.apply_continued(vec![(i, p)]);
    } else {
        store.apply(vec![(i, p)]);
        state.started = true;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    mouse: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    cursor: Res<Cursor>,
    mut editor: ResMut<Editor>,
    mut store: ResMut<SpawnStore>,
    heights: Res<Heights>,
) {
    if egui.wants_any_keyboard_input() {
        return;
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if keys.just_pressed(KeyCode::F5) {
        let request = if editor.playing { Request::Stop } else { Request::Play };
        editor.requests.push(request);
    }
    if editor.playing {
        if keys.just_pressed(KeyCode::Escape) {
            editor.requests.push(Request::Stop);
        }
        return;
    }
    if ctrl && keys.just_pressed(KeyCode::KeyS) {
        editor.requests.push(Request::SaveSpawns);
        editor.requests.push(Request::SaveTerrain);
        editor.requests.push(Request::SaveObjects);
        editor.requests.push(Request::SavePaths);
        editor.requests.push(Request::SaveHousing);
    }
    // Objects, Paths and Housing modes have their own undo (world_objects, paths, housing)
    if ctrl && keys.just_pressed(KeyCode::KeyZ) && !matches!(editor.mode, Mode::Objects | Mode::Paths | Mode::Housing) {
        if editor.mode == Mode::Terrain && heights.can_undo() {
            editor.requests.push(Request::UndoTerrain);
        } else if store.undo() {
            editor.status("undo");
        }
    }
    if ctrl && keys.just_pressed(KeyCode::KeyY) && !matches!(editor.mode, Mode::Objects | Mode::Paths | Mode::Housing) && store.redo() {
        editor.status("redo");
    }
    if keys.just_pressed(KeyCode::Digit1) && !ctrl {
        editor.mode = Mode::Select;
    }
    if keys.just_pressed(KeyCode::Digit2) && !ctrl {
        editor.mode = Mode::Place;
    }
    if keys.just_pressed(KeyCode::Digit3) && !ctrl {
        editor.mode = Mode::Terrain;
    }
    if keys.just_pressed(KeyCode::Digit4) && !ctrl {
        editor.mode = Mode::Objects;
    }
    if keys.just_pressed(KeyCode::Escape) {
        editor.selected = None;
        if editor.mode == Mode::Place {
            editor.mode = Mode::Select;
        }
    }
    // in Paths and Housing modes Delete / F belong to the route / zone editor
    if matches!(editor.mode, Mode::Paths | Mode::Housing) {
        return;
    }
    let Some(i) = editor.selected.filter(|&i| i < store.spawns.len() && !store.spawns[i].now.deleted) else { return };
    let mut p = store.spawns[i].now;
    if keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace) {
        p.deleted = true;
        store.apply(vec![(i, p)]);
        editor.selected = None;
        editor.status(format!("deleted {} {}", store.spawns[i].kind.label(), p.unit_id));
        return;
    }
    if ctrl && keys.just_pressed(KeyCode::KeyD) {
        let kind = store.spawns[i].kind;
        let mut copy = p;
        copy.pos += Vec3::new(2.0, 0.0, 0.0);
        let index = store.add(kind, copy);
        editor.selected = Some(index);
        editor.status("duplicated");
        return;
    }
    if keys.just_pressed(KeyCode::KeyF) && !ctrl {
        editor.requests.push(Request::Focus(p.pos));
    }
    let mut turn = 0.0;
    if keys.just_pressed(KeyCode::BracketLeft) {
        turn += 15.0;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        turn -= 15.0;
    }
    if keys.pressed(KeyCode::ShiftLeft) && !mouse.pressed(MouseButton::Right) && scroll.delta.y != 0.0 && !cursor.over_ui {
        turn += 5.0 * scroll.delta.y.signum();
    }
    if turn != 0.0 {
        p.yaw = ((p.yaw + turn + 180.0).rem_euclid(360.0)) - 180.0;
        store.apply(vec![(i, p)]);
    }
}

fn draw_selection(editor: Res<Editor>, cursor: Res<Cursor>, store: Res<SpawnStore>, views: Query<&SpawnView>, mut gizmos: Gizmos) {
    if editor.playing {
        return;
    }
    let ring = |gizmos: &mut Gizmos, p: Vec3, r: f32, color: Color| {
        let center = cry_to_bevy(p) + Vec3::Y * 0.1;
        gizmos.circle(Isometry3d::new(center, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)), r, color);
    };
    let radius = |i: usize| views.iter().find(|v| v.index == i).map_or(1.0, |v| v.radius.clamp(0.8, 12.0));
    if let Some(i) = cursor.hover.filter(|&i| Some(i) != editor.selected && editor.mode == Mode::Select) {
        ring(&mut gizmos, store.spawns[i].now.pos, radius(i), Color::srgba(1.0, 1.0, 1.0, 0.6));
    }
    if let Some(i) = editor.selected.filter(|&i| i < store.spawns.len() && !store.spawns[i].now.deleted) {
        let s = &store.spawns[i].now;
        let r = radius(i);
        ring(&mut gizmos, s.pos, r, Color::srgb(1.0, 0.75, 0.1));
        // facing arrow (yaw counter-clockwise from north)
        let a = s.yaw.to_radians();
        let forward = Vec3::new(-a.sin(), a.cos(), 0.0);
        let base = cry_to_bevy(s.pos + Vec3::Z * 0.15);
        gizmos.arrow(base, base + cry_to_bevy(forward) * (r + 1.0), Color::srgb(1.0, 0.75, 0.1));
        gizmos.line(base, base + Vec3::Y * (r * 1.5 + 1.0), Color::srgb(1.0, 0.75, 0.1));
    }
    if editor.mode == Mode::Place
        && let Some(g) = cursor.ground {
            let color = match editor.place.map(|p| p.0) {
                Some(Kind::Npc) => Color::srgb(0.4, 0.8, 1.0),
                Some(Kind::Doodad) => Color::srgb(0.6, 1.0, 0.5),
                None => Color::srgb(1.0, 0.3, 0.3),
            };
            ring(&mut gizmos, g, 1.0, color);
            let base = cry_to_bevy(g);
            gizmos.line(base, base + Vec3::Y * 2.5, color);
        }
}
