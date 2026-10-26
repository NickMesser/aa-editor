//! The editor's free-fly camera: hold the right mouse button to look, WASD / Space / Ctrl to move, Shift fast.

use aa_world::cry_to_bevy;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy_egui::input::EguiWantsInput;

use crate::{Editor, Request};

#[derive(Component)]
pub struct EditorCamera;

/// Fly speed in m/s (the mouse wheel changes it while looking).
#[derive(Resource)]
pub struct FlySpeed(pub f32);

pub fn plugin(app: &mut App) {
    app.insert_resource(FlySpeed(40.0)).add_systems(Update, (fly, focus));
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    egui: Res<EguiWantsInput>,
    cursor: Res<crate::tools::Cursor>,
    editor: Res<Editor>,
    mut speed: ResMut<FlySpeed>,
    mut cam: Query<&mut Transform, With<EditorCamera>>,
) {
    if editor.playing {
        return;
    }
    let Ok(mut t) = cam.single_mut() else { return };
    let looking = mouse.pressed(MouseButton::Right) && !cursor.over_ui;
    if looking {
        let d = motion.delta;
        let (mut yaw, mut pitch, _) = t.rotation.to_euler(EulerRot::YXZ);
        yaw -= d.x * 0.003;
        pitch = (pitch - d.y * 0.003).clamp(-1.55, 1.55);
        t.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
        if scroll.delta.y != 0.0 {
            speed.0 = (speed.0 * 1.2f32.powf(scroll.delta.y.signum())).clamp(2.0, 2000.0);
        }
    }
    if egui.wants_any_keyboard_input() || keys.pressed(KeyCode::ControlLeft) && !looking {
        return; // typing, or a Ctrl shortcut
    }
    let fwd = t.forward().as_vec3();
    let right = t.right().as_vec3();
    let mut dir = Vec3::ZERO;
    for (key, v) in [
        (KeyCode::KeyW, fwd),
        (KeyCode::KeyS, -fwd),
        (KeyCode::KeyD, right),
        (KeyCode::KeyA, -right),
        (KeyCode::Space, Vec3::Y),
        (KeyCode::KeyQ, -Vec3::Y),
        (KeyCode::KeyE, Vec3::Y),
    ] {
        if keys.pressed(key) {
            dir += v;
        }
    }
    if looking && keys.pressed(KeyCode::ControlLeft) {
        dir -= Vec3::Y;
    }
    let fast = if keys.pressed(KeyCode::ShiftLeft) { 8.0 } else { 1.0 };
    t.translation += dir.normalize_or_zero() * speed.0 * fast * time.delta_secs();
}

/// [`Request::Focus`]: look at a cry-space point from 18 m away.
fn focus(mut editor: ResMut<Editor>, mut cam: Query<&mut Transform, With<EditorCamera>>) {
    let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::Focus(_))) else { return };
    let Request::Focus(target) = editor.requests.remove(i) else { return };
    let Ok(mut t) = cam.single_mut() else { return };
    let target = cry_to_bevy(target) + Vec3::Y * 1.0;
    let back = Vec3::new(t.forward().x, 0.0, t.forward().z).normalize_or(Vec3::NEG_Z);
    t.translation = target - back * 16.0 + Vec3::Y * 8.0;
    t.look_at(target, Vec3::Y);
}
