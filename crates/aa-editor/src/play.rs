//! Play: walk the edited world as a character, with the editor's markers and panels out of the way.
//! WASD walk (Shift runs), Space jumps, hold the right mouse button to turn the camera, wheel zooms; Esc / F5 stops.

use std::sync::Arc;

use aa_anim::{Animator, BoundClip, Cal, CharacterDesc, ClipLibrary, Gender, Race};
use aa_cry::scene::CryAssets;
use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

use crate::camera::EditorCamera;
use crate::terrain_edit::Heights;
use crate::{AnimDb, Editor, Request};

#[derive(Component)]
struct Player {
    /// Cry space feet position.
    pos: Vec3,
    /// Compass heading of the body, radians (0 = north).
    heading: f32,
    vz: f32,
    moving: bool,
    clips: Option<(BoundClip, Option<BoundClip>)>,
}

#[derive(Resource)]
struct PlayState {
    editor_camera: Transform,
    /// Orbit: compass yaw, pitch (radians), distance.
    yaw: f32,
    pitch: f32,
    distance: f32,
}

pub fn plugin(app: &mut App) {
    app.add_systems(Update, (requests, (walk, follow).chain()).chain());
}

#[allow(clippy::too_many_arguments)]
fn requests(
    mut commands: Commands,
    mut editor: ResMut<Editor>,
    mut cam: Query<&mut Transform, With<EditorCamera>>,
    players: Query<Entity, With<Player>>,
    state: Option<Res<PlayState>>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    anim_db: Res<AnimDb>,
    clips: Res<ClipLibrary>,
    mut cry: CryAssets,
) {
    let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::Play | Request::Stop)) else { return };
    let request = editor.requests.remove(i);
    let Ok(mut t) = cam.single_mut() else { return };
    match request {
        Request::Play if !editor.playing => {
            // start on the ground in front of the camera
            let eye = bevy_to_cry(t.translation);
            let f = bevy_to_cry(*t.forward());
            let flat = Vec2::new(f.x, f.y).normalize_or(Vec2::Y);
            let mut at = eye + Vec3::new(flat.x, flat.y, 0.0) * 12.0;
            at.z = heights.height_at(&query, at.x, at.y).unwrap_or(eye.z);
            let heading = (-flat.x).atan2(flat.y);
            let guard = anim_db.0.as_ref().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()));
            let Some(game) = guard.as_deref() else {
                editor.status("play needs the character tables of the game database");
                return;
            };
            let assets = match CharacterDesc::default_for(game, Race::Nuian, Gender::Male).and_then(|d| d.resolve(game)) {
                Ok(a) => a,
                Err(e) => {
                    editor.status(format!("play: no character: {e}"));
                    return;
                }
            };
            let skeleton = assets.skeleton.clone();
            let cal = assets.cal.clone();
            let root = match aa_anim::spawn_character(&mut commands, &mut cry, &clips, assets, Transform::IDENTITY) {
                Ok(e) => e,
                Err(e) => {
                    editor.status(format!("play: {e}"));
                    return;
                }
            };
            let clip_pair = aa_anim::pose::shared_rig(&skeleton).ok().zip(Cal::shared(&cal).ok()).and_then(|(rig, cal): (Arc<aa_anim::Rig>, Arc<Cal>)| {
                let idle = crate::preview::idle_clip(&rig, &cal, &clips)?;
                let run = crate::preview::bind(&rig, &cal, &clips, "fist_mo_normal_run_f");
                Some((idle, run))
            });
            commands.entity(root).insert(Player { pos: at, heading, vz: 0.0, moving: false, clips: clip_pair });
            commands.insert_resource(PlayState { editor_camera: *t, yaw: heading, pitch: -0.25, distance: 5.0 });
            editor.playing = true;
            editor.status("playing: WASD walk, Shift run, Space jump, right mouse turn, Esc / F5 stop");
        }
        Request::Stop if editor.playing => {
            for e in &players {
                commands.entity(e).despawn();
            }
            if let Some(state) = state {
                *t = state.editor_camera;
            }
            commands.remove_resource::<PlayState>();
            editor.playing = false;
            editor.status("stopped playing");
        }
        _ => {}
    }
}

fn walk(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    state: Option<Res<PlayState>>,
    mut players: Query<(&mut Player, &mut Transform, Option<&mut Animator>)>,
    mut falling: Local<bool>,
) {
    let Some(state) = state else { return };
    let dt = time.delta_secs().min(0.1);
    for (mut p, mut t, animator) in &mut players {
        let mut input = Vec2::ZERO;
        for (key, v) in [(KeyCode::KeyW, Vec2::Y), (KeyCode::KeyS, -Vec2::Y), (KeyCode::KeyA, -Vec2::X), (KeyCode::KeyD, Vec2::X)] {
            if keys.pressed(key) {
                input += v;
            }
        }
        let moving = input != Vec2::ZERO;
        if moving {
            // camera-relative: forward is the camera's compass heading
            let (s, c) = state.yaw.sin_cos();
            let forward = Vec2::new(-s, c);
            let right = Vec2::new(c, s);
            let dir = (forward * input.y + right * input.x).normalize();
            let speed = if keys.pressed(KeyCode::ShiftLeft) { 9.0 } else { 5.4 };
            p.pos += (dir * speed * dt).extend(0.0);
            p.heading = (-dir.x).atan2(dir.y);
        }
        // a hole has no ground: fall (and stop 30 m down, the editor has no cave collision)
        let surface = heights.height_at(&query, p.pos.x, p.pos.y);
        let ground = match heights.ground_at(&query, p.pos.x, p.pos.y) {
            // under the terrain (fell through a hole and walked on): stay on the cave floor, do not pop up
            Some(g) if p.pos.z < g - 2.0 => g - 30.0,
            Some(g) => g,
            None if heights.is_hole_at(&query, p.pos.x, p.pos.y) => surface.map_or(p.pos.z, |h| h - 30.0),
            None => p.pos.z,
        };
        if p.vz < -10.0 && !*falling {
            info!("play: falling through a hole at {:.1},{:.1} (z {:.1})", p.pos.x, p.pos.y, p.pos.z);
        }
        *falling = p.vz < -10.0;
        if keys.just_pressed(KeyCode::Space) && p.pos.z <= ground + 0.05 {
            p.vz = 6.0;
        }
        p.vz -= 18.0 * dt;
        p.pos.z += p.vz * dt;
        if p.pos.z <= ground {
            p.pos.z = ground;
            p.vz = 0.0;
        }
        t.translation = cry_to_bevy(p.pos);
        t.rotation = Quat::from_rotation_y(p.heading);
        if moving != p.moving {
            p.moving = moving;
            if let (Some(mut a), Some((idle, run))) = (animator, p.clips.clone()) {
                match (moving, run) {
                    (true, Some(run)) => a.play(run, true, 0.15),
                    _ => a.play(idle, true, 0.2),
                }
            }
        }
    }
}

fn follow(
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    state: Option<ResMut<PlayState>>,
    players: Query<&Player>,
    mut cam: Query<&mut Transform, With<EditorCamera>>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
) {
    let Some(mut state) = state else { return };
    let (Ok(p), Ok(mut t)) = (players.single(), cam.single_mut()) else { return };
    if mouse.pressed(MouseButton::Right) {
        state.yaw -= motion.delta.x * 0.004;
        state.pitch = (state.pitch - motion.delta.y * 0.004).clamp(-1.3, 0.9);
    }
    if scroll.delta.y != 0.0 {
        state.distance = (state.distance * 0.9f32.powf(scroll.delta.y.signum())).clamp(1.5, 30.0);
    }
    let target = p.pos + Vec3::Z * 1.7;
    let (s, c) = state.yaw.sin_cos();
    let forward = Vec3::new(-s * state.pitch.cos(), c * state.pitch.cos(), state.pitch.sin());
    let mut eye = target - forward * state.distance;
    if let Some(g) = heights.height_at(&query, eye.x, eye.y) {
        eye.z = eye.z.max(g + 0.4);
    }
    *t = Transform::from_translation(cry_to_bevy(eye)).looking_at(cry_to_bevy(target), Vec3::Y);
}
