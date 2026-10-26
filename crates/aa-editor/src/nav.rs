//! Viewport navigation, like most 3D editors: an axis gizmo in the top-right corner of the 3D view (drag it to orbit
//! around the point in the middle of the view, click an axis to look along it), with zoom and pan buttons under it.
//! Also moves the camera for [`Request::Teleport`].

use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::camera::EditorCamera;
use crate::ui::UiArea;
use crate::{Editor, Request};

/// Gizmo radius (logical points).
const R: f32 = 44.0;
/// Seconds an axis click takes to turn the view.
const SNAP_SECS: f32 = 0.25;

#[derive(Resource, Default)]
pub struct Nav {
    /// The point the gizmo orbits around (Bevy space), fixed while a drag lasts.
    pivot: Option<Vec3>,
    /// An axis click turning the view: from, to, progress 0..1.
    anim: Option<(Transform, Transform, f32)>,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Nav>()
        .add_systems(EguiPrimaryContextPass, gizmo.after(crate::ui::panels))
        .add_systems(Update, (teleport, animate));
}

/// Where the view's centre ray meets the ground, else 60 m ahead (Bevy space).
fn pivot(t: &Transform, terrain: &TerrainQuery) -> Vec3 {
    let (o, f) = (t.translation, t.forward().as_vec3());
    let mut d = 1.0;
    while d < 4000.0 {
        let p = bevy_to_cry(o + f * d);
        if terrain.height_at(p.x, p.y).is_some_and(|h| p.z <= h) {
            return o + f * d;
        }
        d += (d * 0.02).max(1.0);
    }
    o + f * 60.0
}

fn yaw_pitch(t: &Transform) -> (f32, f32) {
    let (yaw, pitch, _) = t.rotation.to_euler(EulerRot::YXZ);
    (yaw, pitch)
}

/// The camera at `dist` from `pivot`, looking at it with this yaw / pitch.
fn orbit(pivot: Vec3, dist: f32, yaw: f32, pitch: f32) -> Transform {
    let mut t = Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, yaw, pitch.clamp(-1.55, 1.55), 0.0));
    t.translation = pivot - t.forward().as_vec3() * dist;
    t
}

/// The gizmo's axes: cry direction, colour, label, tooltip.
const AXES: [(Vec3, [u8; 3], &str, &str); 6] = [
    (Vec3::X, [230, 70, 70], "X", "Look from +X (east) to the west"),
    (Vec3::Y, [110, 200, 70], "Y", "Look from +Y (north) to the south"),
    (Vec3::Z, [70, 130, 240], "Z", "Look straight down (top view)"),
    (Vec3::NEG_X, [230, 70, 70], "-X", "Look from -X (west) to the east"),
    (Vec3::NEG_Y, [110, 200, 70], "-Y", "Look from -Y (south) to the north"),
    (Vec3::NEG_Z, [70, 130, 240], "-Z", "Look straight up"),
];

/// The view along an axis clicked on the gizmo, around `pivot`.
fn along(axis: Vec3, t: &Transform, pivot: Vec3) -> Transform {
    let dist = t.translation.distance(pivot).max(5.0);
    let (yaw, pitch) = yaw_pitch(t);
    if axis.z > 0.5 {
        return orbit(pivot, dist, yaw, -1.55);
    }
    if axis.z < -0.5 {
        return orbit(pivot, dist, yaw, 1.55);
    }
    // from +axis towards -axis; keep looking down as much as now (straight down / up becomes 30°)
    let f = cry_to_bevy(-axis);
    let pitch = if pitch.abs() > 1.4 { -0.5 } else { pitch };
    orbit(pivot, dist, f.x.atan2(f.z) + std::f32::consts::PI, pitch)
}

fn gizmo(
    mut contexts: EguiContexts,
    mut area: ResMut<UiArea>,
    mut editor: ResMut<Editor>,
    mut nav: ResMut<Nav>,
    terrain: Res<TerrainQuery>,
    mut cam: Query<&mut Transform, With<EditorCamera>>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let Some(view) = area.view else { return Ok(()) };
    if editor.playing || view.width() < 4.0 * R || view.height() < 4.0 * R {
        return Ok(());
    }
    let Ok(mut t) = cam.single_mut() else { return Ok(()) };
    let origin = egui::pos2(view.right() - 2.0 * R - 12.0, view.top() + 10.0);
    let r = egui::Area::new("nav gizmo".into()).fixed_pos(origin).order(egui::Order::Background).show(&ctx, |ui| {
        let (rect, ball) = ui.allocate_exact_size(egui::vec2(2.0 * R, 2.0 * R), egui::Sense::click_and_drag());
        let c = rect.center();
        let p = ui.painter();
        let pointer = ball.hover_pos();
        if ball.hovered() || ball.dragged() {
            p.circle_filled(c, R, egui::Color32::from_white_alpha(28));
        }
        // axes projected into the view, far ones first
        let inv = t.rotation.inverse();
        let mut axes: Vec<(f32, egui::Pos2, usize)> = AXES
            .iter()
            .enumerate()
            .map(|(i, (a, ..))| {
                let v = inv * cry_to_bevy(*a);
                (v.z, c + egui::vec2(v.x, -v.y) * (R - 11.0), i)
            })
            .collect();
        axes.sort_by(|a, b| a.0.total_cmp(&b.0));
        let hit = pointer.and_then(|m| axes.iter().rev().find(|(_, q, _)| q.distance(m) <= 10.0).map(|a| a.2));
        for &(_, q, i) in &axes {
            let (_, [r, g, b], label, _) = AXES[i];
            let col = egui::Color32::from_rgb(r, g, b);
            let hot = hit == Some(i);
            if i < 3 {
                p.line_segment([c, q], egui::Stroke::new(2.0, col));
                p.circle_filled(q, if hot { 10.0 } else { 9.0 }, if hot { col.gamma_multiply(1.3) } else { col });
                p.text(q, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), egui::Color32::BLACK);
            } else {
                p.circle_filled(q, 7.5, col.gamma_multiply(if hot { 0.8 } else { 0.35 }));
                p.circle_stroke(q, 7.5, egui::Stroke::new(1.0, col));
                if hot {
                    p.text(q, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(9.0), egui::Color32::WHITE);
                }
            }
        }
        let ball = match hit {
            Some(i) => ball.on_hover_text(AXES[i].3),
            None => ball.on_hover_text("Drag to orbit around the middle of the view · click an axis to look along it"),
        };
        if ball.drag_started() {
            nav.pivot = Some(pivot(&t, &terrain));
            nav.anim = None;
        }
        if ball.dragged()
            && let Some(pv) = nav.pivot
        {
            let d = ball.drag_delta();
            let (yaw, pitch) = yaw_pitch(&t);
            *t = orbit(pv, t.translation.distance(pv), yaw - d.x * 0.01, pitch - d.y * 0.01);
        }
        if ball.clicked()
            && let Some(i) = hit
        {
            let pv = pivot(&t, &terrain);
            nav.anim = Some((*t, along(AXES[i].0, &t, pv), 0.0));
        }

        // zoom and pan: drag the buttons
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(R - 30.0);
            for (icon, tip, pan) in [("🔍", "Drag up / down to zoom", false), ("✋", "Drag to pan the view", true)] {
                let (rect, b) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::drag());
                let active = b.hovered() || b.dragged();
                ui.painter().circle_filled(rect.center(), 13.0, egui::Color32::from_white_alpha(if active { 60 } else { 28 }));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(14.0), egui::Color32::WHITE);
                let b = b.on_hover_text(tip);
                if b.drag_started() {
                    nav.pivot = Some(pivot(&t, &terrain));
                    nav.anim = None;
                }
                if b.dragged()
                    && let Some(pv) = nav.pivot
                {
                    let d = b.drag_delta();
                    let dist = t.translation.distance(pv).max(2.0);
                    if pan {
                        let step = (t.right().as_vec3() * -d.x + t.up().as_vec3() * d.y) * dist * 0.002;
                        t.translation += step;
                        nav.pivot = Some(pv + step);
                    } else {
                        let f = t.forward().as_vec3();
                        let new = (dist * (1.0 + d.y * 0.01)).clamp(2.0, 20000.0);
                        t.translation = pv - f * new;
                    }
                }
            }
            if ui.add(egui::Button::new("🗺").min_size(egui::vec2(26.0, 26.0)).corner_radius(13.0)).on_hover_text("Teleport / map (M)").clicked() {
                editor.requests.push(Request::OpenTeleport);
            }
        });
    });
    area.add(r.response.rect, ctx.pixels_per_point());
    Ok(())
}

fn animate(time: Res<Time>, mut nav: ResMut<Nav>, mut cam: Query<&mut Transform, With<EditorCamera>>) {
    let Some((from, to, k)) = nav.anim.as_mut() else { return };
    let Ok(mut t) = cam.single_mut() else { return };
    *k = (*k + time.delta_secs() / SNAP_SECS).min(1.0);
    let s = *k * *k * (3.0 - 2.0 * *k);
    t.translation = from.translation.lerp(to.translation, s);
    t.rotation = from.rotation.slerp(to.rotation, s);
    if *k >= 1.0 {
        nav.anim = None;
    }
}

/// [`Request::Teleport`]: look down at a cry-space point (`z` NaN: the ground there) from about 70 m away;
/// [`Request::View`]: from the given distance.
fn teleport(mut editor: ResMut<Editor>, mut nav: ResMut<Nav>, terrain: Res<TerrainQuery>, mut cam: Query<&mut Transform, With<EditorCamera>>) {
    let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::Teleport(_) | Request::View(..))) else { return };
    let Ok(mut t) = cam.single_mut() else { return };
    let (yaw, _) = yaw_pitch(&t);
    let mut p = match editor.requests.remove(i) {
        Request::View(p, dist) => {
            nav.anim = None;
            *t = orbit(cry_to_bevy(p), dist.max(20.0), yaw, -0.75);
            return;
        }
        Request::Teleport(p) => p,
        _ => return,
    };
    if p.z.is_nan() {
        p.z = terrain.height_at(p.x, p.y).unwrap_or(100.0).max(100.0);
    }
    nav.anim = None;
    *t = orbit(cry_to_bevy(p), 70.0, yaw, -0.6);
    editor.status(format!("teleported to {:.0}, {:.0}, {:.0}", p.x, p.y, p.z));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_views_look_along_the_axis() {
        let t = orbit(Vec3::ZERO, 50.0, 0.3, -0.4);
        for (axis, ..) in AXES {
            let v = along(axis, &t, Vec3::ZERO);
            let f = bevy_to_cry(v.forward().as_vec3());
            // looking from +axis towards -axis: the forward vector points against the axis
            let flat = if axis.z.abs() > 0.5 { f.z * axis.z } else { f.truncate().normalize().dot(axis.truncate()) };
            assert!(flat < -0.99, "{axis:?}: forward {f:?}");
            assert!((v.translation.length() - 50.0).abs() < 0.01);
        }
    }
}
