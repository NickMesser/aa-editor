//! How a humanoid NPC looks: the 28 equipment slots and face / hair / skin parameters the server sends for it, built
//! from the game database the same way, then turned into an aa-anim [`CharacterDesc`] like the client does.
//!
//! Server (`aaemu-game` `systems/npcs/npc/data.rs`, `manager.rs`):
//!
//! * armour: `npcs.equip_cloths_id` -> `equip_pack_cloths` (slots head 0, neck 1, chest 2, waist 3, legs 4, hands 5,
//!   feet 6, arms 7, back 8, undershirt 13, underpants 14, cosplay 27); weapons `equip_pack_weapons` (15..18);
//! * body parts: the last `item_body_parts` row per `slot_type_id` (23 face .. 29 beard) of `npcs.model_id` goes to
//!   slot `19 + slot_type - 23`; hair comes from the look below;
//! * look: `npcs.total_custom_id` -> `total_character_customs` (hair, hair / skin colour, the face block); without one
//!   (and not a monster / animal faction) the server picks a random row of the race's params model whose hair is one
//!   of the model's hair items. The editor picks a fixed row per NPC id so the preview does not change.
//!
//! Client (`aa-client` `ingame/appearance.rs` `describe` / `resolve`): face and hair are body-part items, the hair
//! colour and skin colour come from the look when they belong to the model, the face block becomes the
//! [`Customization`], and the worn armour (underwear first) becomes `Costume::Items` for the items that have meshes for
//! this model.

use aa_anim::custom::Customization;
use aa_anim::db::Decal;
use aa_anim::{CharacterAssets, CharacterDesc, Costume, GameDb, Gender, Race};
use rusqlite::{Connection, OptionalExtension, params};

pub mod slot {
    pub const HEAD: usize = 0;
    pub const NECK: usize = 1;
    pub const CHEST: usize = 2;
    pub const WAIST: usize = 3;
    pub const LEGS: usize = 4;
    pub const HANDS: usize = 5;
    pub const FEET: usize = 6;
    pub const ARMS: usize = 7;
    pub const BACK: usize = 8;
    pub const UNDERSHIRT: usize = 13;
    pub const UNDERPANTS: usize = 14;
    pub const MAINHAND: usize = 15;
    pub const OFFHAND: usize = 16;
    pub const RANGED: usize = 17;
    pub const MUSICAL: usize = 18;
    pub const FACE: usize = 19;
    pub const HAIR: usize = 20;
    pub const COSPLAY: usize = 27;
}

/// Worn armour in drawing order (underwear first, outer layers last), then the costume.
const WORN: [usize; 12] = [
    slot::UNDERPANTS,
    slot::UNDERSHIRT,
    slot::LEGS,
    slot::CHEST,
    slot::WAIST,
    slot::HANDS,
    slot::FEET,
    slot::ARMS,
    slot::NECK,
    slot::HEAD,
    slot::BACK,
    slot::COSPLAY,
];

/// The face / hair / skin parameters of a `total_character_customs` row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub id: u32,
    pub hair: u32,
    pub hair_color: u32,
    pub skin_color: u32,
    pub customization: Customization,
}

/// What the server would send for an NPC.
#[derive(Clone, Debug, PartialEq)]
pub struct NpcLook {
    pub model_id: u32,
    /// Item template per equipment slot (0 = empty).
    pub slots: [u32; 28],
    pub look: Option<Look>,
}

fn opt_u32(v: Option<i64>) -> u32 {
    v.unwrap_or(0).max(0) as u32
}

fn look_row(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Option<Look> {
    conn.query_row(sql, p, |r| {
        let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
        let u = |i: usize| -> rusqlite::Result<u32> { Ok(opt_u32(r.get::<_, Option<i64>>(i)?)) };
        let mut decals = Vec::new();
        if u(4)? != 0 {
            decals.push(Decal { asset_id: u(4)?, weight: f(5)?, scale: f(6)?, rotate: f(7)?, move_x: u(8)? as i32, move_y: u(9)? as i32 });
        }
        for k in 0..4 {
            let id = u(10 + k)?;
            if id != 0 {
                decals.push(Decal { asset_id: id, weight: f(14 + k)?, ..Default::default() });
            }
        }
        let modifier = match r.get::<_, rusqlite::types::Value>(25)? {
            rusqlite::types::Value::Blob(b) => b,
            rusqlite::types::Value::Text(t) => t.into_bytes(),
            _ => Vec::new(),
        };
        Ok(Look {
            id: u(0)?,
            hair: u(1)?,
            hair_color: u(2)?,
            skin_color: u(3)?,
            customization: Customization {
                modifier,
                normal_map: u(18)?,
                normal_map_weight: f(19)?,
                lip_color: u(20)?,
                left_pupil_color: u(21)?,
                right_pupil_color: u(22)?,
                eyebrow_color: u(23)?,
                deco_color: u(24)?,
                decals,
            },
        })
    })
    .optional()
    .ok()
    .flatten()
}

const LOOK_COLUMNS: &str = "id, hair_id, hair_color_id, skin_color_id, \
    face_movable_decal_asset_id, face_movable_decal_weight, face_movable_decal_scale, face_movable_decal_rotate, \
    face_movable_decal_move_x, face_movable_decal_move_y, \
    face_fixed_decal_asset_0_id, face_fixed_decal_asset_1_id, face_fixed_decal_asset_2_id, face_fixed_decal_asset_3_id, \
    face_fixed_decal_asset_0_weight, face_fixed_decal_asset_1_weight, face_fixed_decal_asset_2_weight, face_fixed_decal_asset_3_weight, \
    face_normal_map_id, face_normal_map_weight, lip_color, left_pupil_color, right_pupil_color, eyebrow_color, deco_color, modifier";

/// Factions the server gives no random look (`manager.rs` `load_custom`).
const FACTION_MONSTROSITY: u32 = 115;
const FACTION_ANIMAL: u32 = 116;

/// The params model of a random look (`manager.rs` `load_custom`): by `npcs.char_race_id` and the model's gender
/// (`characters.char_gender_id`, 1 = male); 0 = none.
fn params_model(conn: &Connection, char_race: u32, model_id: u32) -> u32 {
    let actor: bool = conn
        .query_row("SELECT sub_type = 'ActorModel' FROM models WHERE id = ?1", params![model_id], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
        .unwrap_or(false);
    let gender: Option<i64> = conn
        .query_row("SELECT char_gender_id FROM characters WHERE model_id = ?1 LIMIT 1", params![model_id], |r| r.get(0))
        .optional()
        .ok()
        .flatten();
    let male = gender.unwrap_or(0) == 1;
    let pick = |m: u32, f: u32| if male { m } else { f };
    let id = match char_race as u8 {
        0 | 1 => pick(10, 11),
        4 => pick(16, 17),
        5 => pick(18, 19),
        6 => pick(20, 21),
        _ => 0,
    };
    if actor { id } else { 0 }
}

/// The equipment and look of NPC `npc_id`; `None` when it does not exist.
pub fn load(conn: &Connection, npc_id: u32) -> Option<NpcLook> {
    let (model_id, cloths, weapons, custom, faction, char_race) = conn
        .query_row(
            "SELECT model_id, equip_cloths_id, equip_weapons_id, total_custom_id, faction_id, char_race_id FROM npcs WHERE id = ?1",
            params![npc_id],
            |r| {
                Ok((
                    opt_u32(r.get(0)?),
                    opt_u32(r.get(1)?),
                    opt_u32(r.get(2)?),
                    opt_u32(r.get(3)?),
                    opt_u32(r.get(4)?),
                    opt_u32(r.get(5)?),
                ))
            },
        )
        .optional()
        .ok()
        .flatten()?;
    let mut slots = [0u32; 28];
    if cloths != 0 {
        let row: Option<[u32; 12]> = conn
            .query_row(
                "SELECT headgear_id, necklace_id, shirt_id, belt_id, pants_id, glove_id, shoes_id, bracelet_id, back_id, \
                 undershirt_id, underpants_id, cosplay_id FROM equip_pack_cloths WHERE id = ?1",
                params![cloths],
                |r| {
                    let mut v = [0u32; 12];
                    for (i, x) in v.iter_mut().enumerate() {
                        *x = opt_u32(r.get(i)?);
                    }
                    Ok(v)
                },
            )
            .optional()
            .ok()
            .flatten();
        if let Some(v) = row {
            for (i, s) in [0, 1, 2, 3, 4, 5, 6, 7, 8, slot::UNDERSHIRT, slot::UNDERPANTS, slot::COSPLAY].into_iter().enumerate() {
                slots[s] = v[i];
            }
        }
    }
    if weapons != 0 {
        let row: Option<[u32; 4]> = conn
            .query_row(
                "SELECT mainhand_id, offhand_id, ranged_id, musical_id FROM equip_pack_weapons WHERE id = ?1",
                params![weapons],
                |r| Ok([opt_u32(r.get(0)?), opt_u32(r.get(1)?), opt_u32(r.get(2)?), opt_u32(r.get(3)?)]),
            )
            .optional()
            .ok()
            .flatten();
        if let Some(v) = row {
            slots[slot::MAINHAND..=slot::MUSICAL].copy_from_slice(&v);
        }
    }
    // body parts: the last row per slot type
    if let Ok(mut q) = conn.prepare("SELECT slot_type_id, item_id FROM item_body_parts WHERE model_id = ?1 ORDER BY id") {
        if let Ok(rows) = q.query_map(params![model_id], |r| Ok((opt_u32(r.get(0)?), opt_u32(r.get(1)?)))) {
            for (slot_type, item) in rows.flatten() {
                if (23..=29).contains(&slot_type) && slot_type != 24 {
                    slots[19 + (slot_type - 23) as usize] = item;
                }
            }
        }
    }
    let look = if custom != 0 {
        look_row(conn, &format!("SELECT {LOOK_COLUMNS} FROM total_character_customs WHERE id = ?1"), params![custom])
    } else if faction != FACTION_MONSTROSITY && faction != FACTION_ANIMAL && params_model(conn, char_race, model_id) != 0 {
        // the server's random pick, made stable: the row at `npc_id % count`
        let model = params_model(conn, char_race, model_id);
        let hairs = format!("SELECT item_id FROM item_body_parts WHERE model_id = {model_id} AND slot_type_id = 24");
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM total_character_customs WHERE model_id = ?1 AND hair_id IN ({hairs})"), params![model], |r| r.get(0))
            .unwrap_or(0);
        (count > 0)
            .then(|| {
                look_row(
                    conn,
                    &format!("SELECT {LOOK_COLUMNS} FROM total_character_customs WHERE model_id = ?1 AND hair_id IN ({hairs}) ORDER BY id LIMIT 1 OFFSET ?2"),
                    params![model, npc_id as i64 % count],
                )
            })
            .flatten()
    } else {
        None
    };
    if let Some(l) = &look {
        slots[slot::HAIR] = l.hair;
    }
    Some(NpcLook { model_id, slots, look })
}

/// Whether hair style `item` can be drawn by the 1.2 client data: the colour maps of its material are in the pak
/// (the server's newer npc-only styles have meshes but no textures).
fn hair_drawable(db: &GameDb, model: u32, item: u32) -> bool {
    let Ok(part) = db.body_part(item) else { return false };
    let material = db
        .hair_colors_for(model, part.asset_ids[0])
        .ok()
        .and_then(|c| c.into_iter().next())
        .map(|c| c.material)
        .unwrap_or_else(|| part.path.rsplit_once('.').map_or(part.path.clone(), |(stem, _)| format!("{stem}.mtl")));
    let Ok(mtl) = aa_cry::material::Material::load(&material) else { return true };
    let pak = aa_pak::Pak::global();
    let subs = if mtl.sub_materials.is_empty() { std::slice::from_ref(&mtl) } else { &mtl.sub_materials[..] };
    subs.iter().filter_map(|m| m.texture(&aa_cry::material::TextureMap::Diffuse)).all(|t| pak.exists(&t.pak_path()))
}

/// The hair colour row of style `hair` with the same name as colour `id`.
fn same_colour(db: &GameDb, model: u32, id: u32, hair: Option<u32>) -> Option<u32> {
    let name = db.hair_color(id).ok()?.name;
    let asset = db.body_part(hair?).ok()?.asset_ids[0];
    db.hair_colors_for(model, asset).ok()?.into_iter().find(|c| c.name == name).map(|c| c.id)
}

/// Item `id` with meshes for `model`, all in the pak: its visuals.
fn visuals(db: &GameDb, model: u32, id: u32) -> Option<Vec<aa_anim::db::ItemVisual>> {
    let pak = aa_pak::Pak::global();
    db.item_visuals(id, model).ok().filter(|v| !v.is_empty() && v.iter().all(|x| pak.exists(&x.path)))
}

/// The character description of a humanoid NPC (`aa-client` `appearance::describe`).
pub fn describe(db: &GameDb, race: Race, gender: Gender, npc: &NpcLook) -> Option<CharacterDesc> {
    let base = CharacterDesc::default_for(db, race, gender).ok()?;
    let mut desc = base.clone();
    let model = desc.model_id();
    let id = |s: usize| Some(npc.slots[s]).filter(|&i| i != 0);
    desc.face = id(slot::FACE).filter(|i| db.body_part(*i).is_ok());
    if let Some(item) = id(slot::HAIR) {
        desc.hair = db.body_part(item).is_ok().then_some(item);
    }
    match npc.look.as_ref() {
        Some(l) if l.hair_color != 0 && db.hair_color(l.hair_color).is_ok() => desc.hair_color = Some(l.hair_color),
        _ if desc.hair != base.hair => desc.hair_color = None,
        _ => {}
    }
    if desc.hair.is_some_and(|item| Some(item) != base.hair && !hair_drawable(db, model, item)) {
        desc.hair_color = desc.hair_color.and_then(|c| same_colour(db, model, c, base.hair)).or(base.hair_color);
        desc.hair = base.hair;
    }
    if let Some(l) = npc.look.as_ref() {
        if l.skin_color != 0 && db.skin_color(l.skin_color).is_ok_and(|s| s.model_id == model) {
            desc.skin = l.skin_color;
        }
        desc.customization = l.customization.clone();
    }
    desc.costume = Costume::Items(WORN.into_iter().filter_map(id).filter(|i| visuals(db, model, *i).is_some()).collect());
    Some(desc)
}

/// Resolves `desc` with the hair variant of the rendered headgear (`appearance::resolve`).
pub fn resolve(db: &GameDb, desc: &CharacterDesc, npc: &NpcLook) -> aa_anim::Result<CharacterAssets> {
    let model = desc.model_id();
    let detail = match &desc.costume {
        Costume::Items(rendered) => [slot::COSPLAY, slot::HEAD].into_iter().find_map(|s| {
            let item = npc.slots[s];
            if item == 0 || !rendered.contains(&item) {
                return None;
            }
            let v = visuals(db, model, item)?;
            let d = v[0].detail;
            v.iter().all(|x| x.detail == d).then_some(d)
        }),
        _ => None,
    };
    match detail {
        Some(d) => desc.resolve_with_headgear(db, Some(d)),
        None => desc.resolve(db),
    }
}

/// A weapon worn on the body (the relaxed stance): model, attachment bone of the base skeleton and its place there.
#[derive(Clone, Debug, PartialEq)]
pub struct Carried {
    pub path: String,
    pub bone: &'static str,
    pub transform: bevy::prelude::Transform,
}

/// `item_assets.attachment_offset_rot_*` degrees, CryEngine's `SetRotationXYZ` order (`Rz * Ry * Rx`).
fn attachment_rotation(degrees: [f32; 3]) -> bevy::prelude::Quat {
    let [rx, ry, rz] = degrees.map(f32::to_radians);
    bevy::prelude::Quat::from_euler(bevy::prelude::EulerRot::ZYX, rz, ry, rx)
}

/// The weapons an NPC carries at peace (`aa-client` `ingame/weapons.rs` `carried`): main / off hand weapons and the
/// bow on their sheath bones. Shields and instruments are only out while used, so they are left out.
pub fn carried(conn: &Connection, npc: &NpcLook) -> Vec<Carried> {
    let mut out = Vec::new();
    for s in [slot::MAINHAND, slot::OFFHAND, slot::RANGED] {
        let item = npc.slots[s];
        if item == 0 {
            continue;
        }
        let row = conn
            .query_row(
                "SELECT ia.path, h.code, ia.attachment_offset_pos_x, ia.attachment_offset_pos_y, ia.attachment_offset_pos_z, \
                 ia.attachment_offset_rot_x, ia.attachment_offset_rot_y, ia.attachment_offset_rot_z, w.worn_scale, h.slot_type_id, \
                 h.pose_id, ia.allow_mirror FROM item_weapons w JOIN item_assets ia ON ia.id = w.asset_id \
                 JOIN holdables h ON h.id = w.holdable_id WHERE w.item_id = ?1",
                params![item],
                |r| {
                    let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        [f(2)?, f(3)?, f(4)?],
                        [f(5)?, f(6)?, f(7)?],
                        f(8)?,
                        r.get::<_, Option<i64>>(9)?.unwrap_or(0) == 16,
                        r.get::<_, Option<i64>>(10)?.unwrap_or(0) == 4,
                        matches!(r.get::<_, Option<String>>(11)?.as_deref(), Some("t" | "T" | "1")),
                    ))
                },
            )
            .optional()
            .ok()
            .flatten();
        let Some((path, code, mut offset, mut rotation, scale, two_handed, staff, mirror)) = row else { continue };
        if code == "shield" || code.ends_with("_instrument") {
            continue;
        }
        if s == slot::OFFHAND && mirror {
            offset[0] = -offset[0];
            rotation[1] = -rotation[1];
            rotation[2] = -rotation[2];
        }
        let bone = match (code.as_str(), s) {
            _ if staff => "item_staff_l",
            ("bow", _) => "item_bow_l",
            _ if two_handed => "item_back_r",
            (_, slot::OFFHAND) => "item_side_r",
            (_, slot::MAINHAND) => "item_side_l",
            _ => "item_back_r",
        };
        let path = path.trim().replace('\\', "/").to_ascii_lowercase();
        let path = path.strip_prefix("game/").map(str::to_string).unwrap_or(path);
        out.push(Carried {
            path,
            bone,
            transform: bevy::prelude::Transform {
                translation: offset.into(),
                rotation: attachment_rotation(rotation),
                scale: bevy::prelude::Vec3::splat(if scale > 0.0 { scale } else { 1.0 }),
            },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temple_priestess_wears_her_own_clothes() {
        let path = "D:/aaemu-live/rs-game/Data/compact.sqlite3";
        if !std::path::Path::new(path).exists() {
            return;
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let npc = load(&conn, 502).unwrap();
        assert_eq!(npc.model_id, 11);
        assert_eq!([npc.slots[0], npc.slots[2], npc.slots[4], npc.slots[6], npc.slots[15]], [25861, 25862, 25863, 25864, 26033]);
        assert_eq!(npc.slots[slot::FACE], 19839);
        let look = npc.look.as_ref().expect("a random look for a non-monster");
        assert_eq!(npc.slots[slot::HAIR], look.hair);
        assert_eq!(load(&conn, 502), Some(npc.clone()), "the pick is stable");
        let weapons = carried(&conn, &npc);
        assert_eq!(weapons.len(), 1, "{weapons:?}");
        assert_eq!(weapons[0].bone, "item_side_l");
        if aa_pak::Pak::global().is_empty() {
            return;
        }
        let db = GameDb::open().unwrap();
        let desc = describe(&db, Race::Nuian, Gender::Female, &npc).unwrap();
        let Costume::Items(items) = &desc.costume else { panic!() };
        assert!(items.contains(&25862), "{items:?}");
        resolve(&db, &desc, &npc).unwrap();
    }
}
