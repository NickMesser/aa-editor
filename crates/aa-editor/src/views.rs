//! The spawns around the camera as entities: a marker for each, and its model once loaded (a few per frame, nearest
//! first, within a time budget so the editor stays responsive while a town fills in).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use aa_cry::scene::CryAssets;
use aa_world::{bevy_to_cry, cry_to_bevy};
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;

use crate::camera::EditorCamera;
use crate::db::GameData;
use crate::preview::Previewer;
use crate::spawns::{Kind, Placement, SpawnStore};
use crate::{AnimDb, Editor};

/// A spawn shown in the world.
#[derive(Component)]
pub struct SpawnView {
    pub index: usize,
    /// Horizontal pick radius (metres), from the model's bounds once loaded.
    pub radius: f32,
    shown: Placement,
    model: Option<Entity>,
    /// Frames left to measure the model's bounds.
    measure: u32,
}

#[derive(Resource, Default)]
struct Views {
    by_index: HashMap<usize, Entity>,
    center: Option<Vec3>,
    generation: u64,
    settings: (u32, bool, bool),
    queue: Vec<usize>,
    /// Model references that could not be loaded.
    failed: HashSet<String>,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Views>().add_systems(Update, (sync, load_models, crate::preview::attach_carried, measure, markers).chain());
}

/// Cap on the spawns shown at once.
const MAX_VIEWS: usize = 2500;

fn transform(kind: Kind, p: &Placement, game: &GameData) -> Transform {
    let scale = match kind {
        Kind::Npc => game.npc_scale(p.unit_id),
        Kind::Doodad => p.scale,
    };
    Transform { translation: cry_to_bevy(p.pos), rotation: p.rotation(), scale: Vec3::splat(scale) }
}

fn sync(
    mut commands: Commands,
    editor: Res<Editor>,
    store: Res<SpawnStore>,
    game: Res<GameData>,
    mut views: ResMut<Views>,
    cam: Query<&GlobalTransform, With<EditorCamera>>,
    mut shown: Query<(&mut SpawnView, &mut Transform)>,
) {
    let Ok(cam) = cam.single() else { return };
    let center = bevy_to_cry(cam.translation());
    let settings = (editor.view_radius as u32, editor.show_npcs, editor.show_doodads);
    let moved = views.center.is_none_or(|c| c.truncate().distance(center.truncate()) > 15.0);
    if !moved && views.generation == store.generation && views.settings == settings {
        return;
    }
    views.center = Some(center);
    views.generation = store.generation;
    views.settings = settings;
    let mut wanted: Vec<usize> = store
        .near(center, editor.view_radius)
        .filter(|&i| match store.spawns[i].kind {
            Kind::Npc => editor.show_npcs,
            Kind::Doodad => editor.show_doodads,
        })
        .collect();
    if wanted.len() > MAX_VIEWS {
        wanted.sort_by(|&a, &b| {
            store.spawns[a].now.pos.distance_squared(center).total_cmp(&store.spawns[b].now.pos.distance_squared(center))
        });
        wanted.truncate(MAX_VIEWS);
    }
    let keep: HashSet<usize> = wanted.iter().copied().collect();
    let gone: Vec<usize> = views.by_index.keys().filter(|i| !keep.contains(i)).copied().collect();
    for i in gone {
        if let Some(e) = views.by_index.remove(&i) {
            commands.entity(e).despawn();
        }
    }
    for i in wanted {
        let s = &store.spawns[i];
        match views.by_index.get(&i).copied() {
            Some(e) => {
                let Ok((mut view, mut t)) = shown.get_mut(e) else { continue };
                if view.shown != s.now {
                    *t = transform(s.kind, &s.now, &game);
                    if view.shown.unit_id != s.now.unit_id {
                        if let Some(m) = view.model.take() {
                            commands.entity(m).despawn();
                        }
                        views.queue.push(i);
                    }
                    view.shown = s.now;
                }
            }
            None => {
                let e = commands
                    .spawn((
                        Name::new(format!("{} {}", s.kind.label(), s.now.unit_id)),
                        SpawnView { index: i, radius: 1.0, shown: s.now, model: None, measure: 0 },
                        transform(s.kind, &s.now, &game),
                        Visibility::default(),
                    ))
                    .id();
                views.by_index.insert(i, e);
                views.queue.push(i);
            }
        }
    }
    // nearest last: `load_models` pops from the end
    let spawns = &store.spawns;
    views.queue.sort_by(|&a, &b| {
        spawns[b].now.pos.distance_squared(center).total_cmp(&spawns[a].now.pos.distance_squared(center))
    });
    views.queue.dedup();
}

fn load_models(
    mut commands: Commands,
    mut views: ResMut<Views>,
    store: Res<SpawnStore>,
    mut game: ResMut<GameData>,
    anim_db: Res<AnimDb>,
    clips: Res<aa_anim::ClipLibrary>,
    mut cry: CryAssets,
    mut shown: Query<&mut SpawnView>,
) {
    let started = Instant::now();
    let anim_db = anim_db.0.as_ref().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()));
    let mut previewer = Previewer { cry: &mut cry, clips: &clips, game: anim_db.as_deref() };
    while started.elapsed() < Duration::from_millis(12) {
        let Some(i) = views.queue.pop() else { break };
        let Some(&e) = views.by_index.get(&i) else { continue };
        let Ok(mut view) = shown.get_mut(e) else { continue };
        if view.model.is_some() {
            continue;
        }
        let s = &store.spawns[i];
        let reference = match s.kind {
            Kind::Npc => game.npc_model(s.now.unit_id),
            Kind::Doodad => game.doodad_model(s.now.unit_id),
        };
        let Some(reference) = reference.filter(|r| !views.failed.contains(r)) else { continue };
        let look = (s.kind == Kind::Npc).then(|| game.npc_look(s.now.unit_id)).flatten();
        match previewer.spawn_npc(&mut commands, &reference, look.as_ref()) {
            Some(model) => {
                if let Some(look) = look.as_ref() {
                    Previewer::carry(&mut commands, model, game.npc_carried(look));
                }
                commands.entity(model).insert(ChildOf(e));
                view.model = Some(model);
                view.measure = 40;
            }
            None => {
                debug!("no preview for {reference}");
                views.failed.insert(reference);
            }
        }
    }
}

/// Pick radius from the model's mesh bounds (they exist a frame after the meshes).
fn measure(mut shown: Query<(Entity, &mut SpawnView, &GlobalTransform)>, children: Query<&Children>, bounds: Query<(&Aabb, &GlobalTransform)>) {
    for (e, mut view, root) in &mut shown {
        if view.measure == 0 {
            continue;
        }
        view.measure -= 1;
        let inverse = root.affine().inverse();
        let mut extent: f32 = 0.0;
        let mut found = false;
        for d in children.iter_descendants(e) {
            let Ok((aabb, t)) = bounds.get(d) else { continue };
            found = true;
            let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
            for corner in [Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, -1.0, 1.0), Vec3::new(-1.0, 1.0, -1.0)] {
                let p = inverse.transform_point3(t.transform_point(c + h * corner));
                extent = extent.max(Vec2::new(p.x, p.z).length());
            }
        }
        if found {
            view.radius = (extent * root.scale().x).clamp(0.6, 30.0);
            view.measure = view.measure.min(5);
        }
    }
}

fn markers(editor: Res<Editor>, store: Res<SpawnStore>, shown: Query<&SpawnView>, mut gizmos: Gizmos) {
    if editor.playing {
        return;
    }
    for view in &shown {
        let Some(s) = store.spawns.get(view.index) else { continue };
        let color = if s.changed() {
            Color::srgb(1.0, 0.55, 0.1)
        } else {
            match s.kind {
                Kind::Npc => Color::srgb(0.35, 0.8, 1.0),
                Kind::Doodad => Color::srgb(0.55, 1.0, 0.45),
            }
        };
        let base = cry_to_bevy(s.now.pos);
        let h = if view.model.is_some() { 0.4 } else { 2.0 };
        gizmos.line(base, base + Vec3::Y * h, color);
        if view.model.is_none() {
            gizmos.sphere(Isometry3d::from_translation(base + Vec3::Y * h), 0.25, color);
        }
    }
}
