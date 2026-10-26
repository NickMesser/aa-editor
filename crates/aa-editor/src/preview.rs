//! Model previews: what a spawn looks like in game.
//!
//! * doodads: `doodad_almighties.model`, a `.cgf` or every mesh part of a `prefab://file.xml/name` element;
//! * NPCs: the actor model of `npcs.model_id`: a humanoid body (`characters/<race>/<gender>/nude/...`) is dressed like
//!   the character-create stage (the NPC's own gear is not resolved yet), a creature `.cdf` / `.chr` gets its skin
//!   attachments and idle animation, a static model is spawned as is.
//!
//! The roots are in Bevy space with the feet at the origin, facing game north at identity. Simplified from
//! `aa-client`'s `ingame::{creature, doodad_model, housing::prefab}` (binary-only there).

use std::sync::Arc;

use aa_anim::cdf::{AttachmentKind, BindingKind};
use aa_anim::{Animator, Cal, CharacterDesc, ClipLibrary, GameDb, Gender, Race, SampleOptions};
use aa_cry::CRY_TO_BEVY;
use aa_cry::scene::{CryAssets, ModelOptions, SkeletonBinding, spawn_loaded};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::prelude::*;

/// One model of a prefab, relative to the prefab's origin (cry space).
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub model: String,
    pub material: Option<String>,
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
}

/// The value of attribute `name` in the text of one xml tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let at = tag.find(&key)? + key.len();
    let rest = &tag[at..];
    Some(&rest[..rest.find('"')?])
}

fn vec3(text: &str) -> Option<Vec3> {
    let mut it = text.split(',').map(|v| v.trim().parse::<f32>().ok());
    Some(Vec3::new(it.next()??, it.next()??, it.next()??))
}

fn pak_path(raw: &str) -> Option<String> {
    let p = raw.trim().replace('\\', "/").to_ascii_lowercase();
    let p = p.strip_prefix("game/").unwrap_or(&p).to_string();
    p.contains('.').then_some(p)
}

/// The mesh parts of prefab `name` in the text of its xml library.
pub fn prefab_parts(xml: &str, name: &str) -> Vec<Part> {
    let Some(from) = xml.find(&format!("<Prefab Name=\"{name}\"")) else { return Vec::new() };
    let tail = &xml[from..];
    let prefab = &tail[..tail.find("</Prefab>").unwrap_or(tail.len())];
    let objects = &prefab[..prefab.find("<export>").unwrap_or(prefab.len())];
    let mut out = Vec::new();
    let mut rest = objects;
    while let Some(at) = rest.find("<Object ") {
        rest = &rest[at + 7..];
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        let body = &rest[..rest.find("<Object ").unwrap_or(rest.len())];
        let model = match attr(tag, "Type") {
            Some("Brush") => attr(tag, "Prefab").and_then(pak_path),
            Some("Entity") => match attr(tag, "EntityClass") {
                Some("GeomEntity") => attr(tag, "Geometry").and_then(pak_path),
                _ => body.find("object_Model=\"").and_then(|s| {
                    let v = &body[s + 14..];
                    pak_path(&v[..v.find('"')?])
                }),
            },
            _ => None,
        };
        let Some(model) = model else { continue };
        let rot = attr(tag, "Rotate")
            .and_then(|t| {
                let v: Vec<f32> = t.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                (v.len() == 4).then(|| Quat::from_xyzw(v[1], v[2], v[3], v[0]))
            })
            .filter(|q| q.length_squared() > 1e-6)
            .map(Quat::normalize)
            .unwrap_or(Quat::IDENTITY);
        out.push(Part {
            model,
            material: attr(tag, "Material").filter(|m| !m.trim().is_empty()).map(|m| {
                let m = m.trim().replace('\\', "/").to_ascii_lowercase();
                let m = m.strip_prefix("game/").unwrap_or(&m).to_string();
                if m.ends_with(".mtl") { m } else { format!("{m}.mtl") }
            }),
            pos: attr(tag, "Pos").and_then(vec3).unwrap_or(Vec3::ZERO),
            rot,
            scale: attr(tag, "Scale")
                .and_then(|s| vec3(s).or_else(|| s.trim().parse::<f32>().ok().map(Vec3::splat)))
                .unwrap_or(Vec3::ONE),
        });
    }
    out
}

fn load_prefab(element: &str) -> Vec<Part> {
    let Some((file, name)) = element.strip_prefix("prefab://").and_then(|r| r.rsplit_once('/')) else { return Vec::new() };
    let file = file.replace('\\', "/").to_ascii_lowercase();
    aa_pak::Pak::global().read_string(&file).map(|xml| prefab_parts(&xml, name)).unwrap_or_default()
}

/// Race and gender of a humanoid body file (`objects/characters/<race>/<gender>/nude/...`).
fn humanoid(file: &str) -> Option<(Race, Gender)> {
    let rest = file.split("characters/").nth(1)?;
    let mut it = rest.split('/');
    let folder = it.next()?;
    let race = Race::ALL.into_iter().find(|r| r.folder() == folder)?;
    let gender = match it.next()? {
        "male" => Gender::Male,
        "female" => Gender::Female,
        _ => return None,
    };
    (it.next()? == "nude").then_some((race, gender))
}

/// Everything needed to spawn previews.
pub struct Previewer<'a, 'w> {
    pub cry: &'a mut CryAssets<'w>,
    pub clips: &'a ClipLibrary,
    pub game: Option<&'a GameDb>,
}

impl Previewer<'_, '_> {
    /// Spawns the model `reference` (pak path or `prefab://`); `None` when nothing could be loaded. A humanoid body is
    /// dressed like NPC `npc` (its equipment and look) when given.
    pub fn spawn_npc(&mut self, commands: &mut Commands, reference: &str, npc: Option<&crate::npc_look::NpcLook>) -> Option<Entity> {
        if reference.starts_with("prefab://") {
            return self.spawn_prefab(commands, reference);
        }
        if let Some((race, gender)) = humanoid(reference)
            && let Some(e) = self.spawn_humanoid(commands, race, gender, npc)
        {
            return Some(e);
        }
        if reference.ends_with(".cdf") || reference.ends_with(".chr") {
            return self.spawn_creature(commands, reference);
        }
        let options = ModelOptions::default();
        let loaded = self.cry.load_model(reference, &options).map_err(|e| debug!("preview {reference}: {e}")).ok()?;
        Some(spawn_loaded(commands, reference, &loaded, &options))
    }

    fn spawn_prefab(&mut self, commands: &mut Commands, reference: &str) -> Option<Entity> {
        let parts = load_prefab(reference);
        if parts.is_empty() {
            return None;
        }
        let root = commands.spawn((Name::new(reference.to_string()), Transform::IDENTITY, Visibility::default())).id();
        let mut any = false;
        for part in parts {
            let options = ModelOptions { material: part.material.clone().filter(|m| aa_pak::Pak::global().exists(m)), ..default() };
            let Ok(loaded) = self.cry.load_model(&part.model, &options) else { continue };
            let model = spawn_loaded(commands, &part.model, &loaded, &options);
            let holder = commands
                .spawn((
                    Transform {
                        translation: cry_to_bevy(part.pos),
                        rotation: cry_rot_to_bevy(part.rot),
                        scale: Vec3::new(part.scale.x, part.scale.z, part.scale.y),
                    },
                    Visibility::default(),
                    ChildOf(root),
                ))
                .id();
            commands.entity(model).insert(ChildOf(holder));
            any = true;
        }
        if !any {
            commands.entity(root).despawn();
        }
        any.then_some(root)
    }

    fn spawn_humanoid(&mut self, commands: &mut Commands, race: Race, gender: Gender, npc: Option<&crate::npc_look::NpcLook>) -> Option<Entity> {
        let game = self.game?;
        let dressed = npc.and_then(|npc| {
            let desc = crate::npc_look::describe(game, race, gender, npc)?;
            crate::npc_look::resolve(game, &desc, npc).map_err(|e| debug!("npc look: {e}")).ok()
        });
        let assets = match dressed {
            Some(a) => a,
            None => CharacterDesc::default_for(game, race, gender).and_then(|d| d.resolve(game)).ok()?,
        };
        aa_anim::spawn_character(commands, self.cry, self.clips, assets, Transform::IDENTITY)
            .map_err(|e| debug!("humanoid preview {race:?} {gender:?}: {e}"))
            .ok()
    }

    /// Hangs `items` on the bones of the character `root` once its skeleton exists ([`attach_carried`]).
    pub fn carry(commands: &mut Commands, root: Entity, items: Vec<crate::npc_look::Carried>) {
        if !items.is_empty() {
            commands.entity(root).insert(CarriedItems(items));
        }
    }

    fn spawn_creature(&mut self, commands: &mut Commands, file: &str) -> Option<Entity> {
        let (main, main_material, skins) = if file.ends_with(".cdf") {
            let cdf = aa_anim::Cdf::load(file).map_err(|e| debug!("creature {file}: {e}")).ok()?;
            let skins: Vec<(String, Option<String>)> = cdf
                .attachments
                .iter()
                .filter(|a| a.kind == AttachmentKind::Skin && a.binding_kind == BindingKind::Object && !a.binding.is_empty() && a.flags & 1 == 0)
                .map(|a| (a.binding.clone(), a.material.clone()))
                .collect();
            (cdf.model, cdf.material, skins)
        } else {
            (file.to_string(), None, Vec::new())
        };
        if main.is_empty() {
            return None;
        }
        let root = commands.spawn((Name::new(main.clone()), Transform::IDENTITY, Visibility::default())).id();
        let Some(rig) = aa_anim::pose::shared_rig(&main).ok().filter(|r| !r.is_empty()) else {
            let options = ModelOptions { material: main_material, ..default() };
            let Ok(loaded) = self.cry.load_model(&main, &options) else {
                commands.entity(root).despawn();
                return None;
            };
            let model = spawn_loaded(commands, &main, &loaded, &options);
            commands.entity(model).insert(ChildOf(root));
            return Some(root);
        };
        let model_root = commands.spawn((Name::new("model"), Transform::from_rotation(CRY_TO_BEVY), Visibility::default(), ChildOf(root))).id();
        let bones = aa_anim::spawn_bones(commands, model_root, &rig);
        let mut paths = vec![(main.clone(), main_material)];
        paths.extend(skins);
        let mut any = false;
        for (path, material) in paths {
            let (path, material) = if path.ends_with(".cdf") {
                match aa_anim::Cdf::load(&path) {
                    Ok(c) => (c.model, c.material.or(material)),
                    Err(_) => continue,
                }
            } else {
                (path, material)
            };
            let options = ModelOptions {
                material,
                skeleton: Some(SkeletonBinding { skeleton: rig.skeleton.clone(), joints: bones.clone() }),
                cry_space: true,
                ..default()
            };
            let Ok(loaded) = self.cry.load_model(&path, &options) else { continue };
            let part = spawn_loaded(commands, &path, &loaded, &options);
            commands.entity(part).insert(ChildOf(model_root));
            any = true;
        }
        if !any {
            commands.entity(root).despawn();
            return None;
        }
        let mut animator = Animator::new(rig.clone(), bones);
        let cal = format!("{}.cal", main.trim_end_matches(".chr"));
        if let Some(idle) = Cal::shared(&cal).ok().and_then(|cal| idle_clip(&rig, &cal, self.clips)) {
            animator.play(idle, true, 0.0);
        }
        commands.entity(root).insert(animator);
        Some(root)
    }
}

/// Weapons waiting for their character's bones.
#[derive(Component)]
pub struct CarriedItems(Vec<crate::npc_look::Carried>);

/// Spawns the carried weapon models under the attachment bones (`aa-client` `weapons::attach_carried`).
pub fn attach_carried(mut commands: Commands, mut cry: CryAssets, characters: Query<(Entity, &Animator, &CarriedItems)>) {
    for (entity, animator, carried) in &characters {
        let bone_of = |name: &str| animator.rig.skeleton.bones.iter().position(|b| b.name == name).and_then(|i| animator.bones.get(i).copied());
        for item in &carried.0 {
            let Some(bone) = bone_of(item.bone) else { continue };
            let options = ModelOptions { cry_space: true, shading: aa_cry::scene::Shading::Full, ..default() };
            let mut parts = Vec::new();
            for (path, material) in weapon_files(&item.path) {
                let options = ModelOptions { material, ..options.clone() };
                let Ok(loaded) = cry.load_model(&path, &options) else { continue };
                parts.push(spawn_loaded(&mut commands, &path, &loaded, &options));
            }
            let model = match parts[..] {
                [] => continue,
                [one] => one,
                _ => commands.spawn((Name::new("weapon"), Transform::default(), Visibility::default())).add_children(&parts).id(),
            };
            commands.entity(model).insert((item.transform, ChildOf(bone)));
        }
        commands.entity(entity).remove::<CarriedItems>();
    }
}

/// The model files of a weapon: one `.cgf`, or a `.cdf` carrier with its object attachments (`weapons::model_files`).
fn weapon_files(path: &str) -> Vec<(String, Option<String>)> {
    if !path.ends_with(".cdf") {
        return vec![(path.to_string(), None)];
    }
    let pak = aa_pak::Pak::global();
    let mtl = |m: &str| if m.ends_with(".mtl") { m.to_string() } else { format!("{m}.mtl") };
    let Ok(cdf) = aa_anim::Cdf::load(path) else {
        let cgf = format!("{}.cgf", path.trim_end_matches(".cdf"));
        return if pak.exists(&cgf) { vec![(cgf, None)] } else { Vec::new() };
    };
    let mut out = Vec::new();
    if !cdf.model.ends_with("/real_torch.chr") && pak.exists(&cdf.model) {
        out.push((cdf.model.clone(), cdf.material.as_deref().map(mtl)));
    }
    for a in &cdf.attachments {
        if a.binding_kind == BindingKind::Object && !a.binding.is_empty() && a.flags & 1 == 0 && pak.exists(&a.binding) {
            out.push((a.binding.clone(), a.material.as_deref().map(mtl)));
        }
    }
    out
}

/// The first idle clip a model's `.cal` has.
pub fn idle_clip(rig: &aa_anim::Rig, cal: &Arc<Cal>, clips: &ClipLibrary) -> Option<aa_anim::BoundClip> {
    ["fist_ba_relaxed_idle", "fist_ba_idle", "fist_ac_stage_idle", "fist_ba_combat_idle"].into_iter().find_map(|n| bind(rig, cal, clips, n))
}

/// Clip `name` of `cal`, bound to `rig` (root motion locked).
pub fn bind(rig: &aa_anim::Rig, cal: &Cal, clips: &ClipLibrary, name: &str) -> Option<aa_anim::BoundClip> {
    let entry = cal.animation(name)?;
    let options = SampleOptions { position_scale: entry.position_scale(cal.base_height), lock_root_motion: true };
    clips.bound(rig, &entry.path, options).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefab_parts_are_read_from_brushes_and_entities() {
        let xml = r#"<PrefabsLibrary><Prefab Name="a.b"><Objects>
<Object Type="Brush" Name="x" Pos="1.5,-2,0.25" Rotate="0.70710671,0,0,0.70710683" Scale="0.88,0.88,0.88" Prefab="Objects\env\barrel.cgf"/>
<Object Type="Entity" Name="d" Pos="0,0,1" EntityClass="AnimObject"><Properties object_Model="game/Objects/door.cga"/></Object>
<Object Type="Entity" Name="l" EntityClass="Light"><Properties Radius="5"/></Object>
</Objects><export><Entity object_Model="ignored.cga"/></export></Prefab></PrefabsLibrary>"#;
        let parts = prefab_parts(xml, "a.b");
        assert_eq!(parts.iter().map(|p| p.model.as_str()).collect::<Vec<_>>(), ["objects/env/barrel.cgf", "objects/door.cga"]);
        assert!((parts[0].rot * Vec3::X - Vec3::Y).length() < 1e-4);
        assert_eq!(parts[1].pos, Vec3::Z);
    }

    #[test]
    fn humanoid_bodies_are_recognised() {
        assert_eq!(humanoid("objects/characters/nuian/female/nude/nu_f.cdf"), Some((Race::Nuian, Gender::Female)));
        assert_eq!(humanoid("objects/characters/people/npc_chef01/nude/npc_chef01.cdf"), None);
    }
}
