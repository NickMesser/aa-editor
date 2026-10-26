//! The game database (`compact.sqlite3`, plain SQLite) the AAEmu server loads at startup: the browser lists
//! (NPCs, doodads, items, quests with their English names), model lookups for the previews, and the row editor.
//!
//! The editor opens the bevy lane's own copy (`D:/aaemu-live-bevy/rs-game/Data/compact.sqlite3`) read-write; the
//! shared `D:/aaemu-live/...` copy is used by other lanes and is never written. Names live in `localized_texts`
//! (`tbl_name`, `tbl_column_name`, `idx`, one column per language).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use bevy::prelude::*;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, params};

/// A table the editor browses and edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Table {
    Npcs,
    Doodads,
    Items,
    Quests,
}

impl Table {
    pub const ALL: [Table; 4] = [Table::Npcs, Table::Doodads, Table::Items, Table::Quests];

    pub fn sql(self) -> &'static str {
        match self {
            Table::Npcs => "npcs",
            Table::Doodads => "doodad_almighties",
            Table::Items => "items",
            Table::Quests => "quest_contexts",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Table::Npcs => "NPCs",
            Table::Doodads => "Doodads",
            Table::Items => "Items",
            Table::Quests => "Quests",
        }
    }
}

/// One browser entry.
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u32,
    /// English name, else the table's own (Korean) `name`.
    pub name: String,
    /// NPC: `model_id`; doodad: unused (0).
    pub model_id: u32,
    /// Doodad: the `model` reference (`cgf://...` / `prefab://...`).
    pub model: String,
    /// Lower-case `"<id> <name>"` for searching.
    search: String,
}

impl Entry {
    pub fn matches(&self, needle: &str) -> bool {
        needle.is_empty() || self.search.contains(needle)
    }
}

/// One column of a row in the row editor.
#[derive(Clone, Debug)]
pub struct Field {
    pub column: String,
    pub text: String,
    pub null: bool,
    pub original: String,
    /// The stored value's type: an edit is written back as the same type when it parses as one.
    kind: ValueKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueKind {
    Null,
    Integer,
    Real,
    Text,
    Blob,
}

impl Field {
    fn value(&self) -> SqlValue {
        let t = self.text.trim();
        match self.kind {
            _ if self.text.is_empty() && self.null => SqlValue::Null,
            ValueKind::Text => SqlValue::Text(self.text.clone()),
            ValueKind::Integer if t.parse::<i64>().is_ok() => SqlValue::Integer(t.parse().unwrap()),
            ValueKind::Real if t.parse::<f64>().is_ok() => SqlValue::Real(t.parse().unwrap()),
            // a NULL (or a number that no longer parses): the most specific type the text has
            _ => match (t.parse::<i64>(), t.parse::<f64>()) {
                (Ok(i), _) => SqlValue::Integer(i),
                (_, Ok(r)) => SqlValue::Real(r),
                _ => SqlValue::Text(self.text.clone()),
            },
        }
    }
}

#[derive(Resource)]
pub struct GameData {
    conn: Mutex<Connection>,
    pub path: String,
    pub writable: bool,
    pub lists: HashMap<Table, Vec<Entry>>,
    by_id: HashMap<Table, HashMap<u32, usize>>,
    actor_files: HashMap<u32, Option<String>>,
}

fn norm(path: &str) -> String {
    let p = path.trim().replace('\\', "/").to_ascii_lowercase();
    p.strip_prefix("game/").map(str::to_string).unwrap_or(p)
}

impl GameData {
    /// Opens `path` read-write (or read-only with `read_only`) and loads the browser lists.
    pub fn open(path: &Path, read_only: bool) -> Result<GameData> {
        let flags = if read_only { OpenFlags::SQLITE_OPEN_READ_ONLY } else { OpenFlags::SQLITE_OPEN_READ_WRITE };
        let conn = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| anyhow::anyhow!("open {}: {e}", path.display()))?;
        // the retail data references ids that do not exist (0 for "none"): enforcing foreign keys would refuse
        // to clone or edit most rows
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        let mut data = GameData {
            conn: Mutex::new(conn),
            path: path.display().to_string(),
            writable: !read_only,
            lists: HashMap::new(),
            by_id: HashMap::new(),
            actor_files: HashMap::new(),
        };
        data.reload_lists()?;
        Ok(data)
    }

    pub fn reload_lists(&mut self) -> Result<()> {
        let started = std::time::Instant::now();
        let conn = self.conn.get_mut().unwrap_or_else(|e| e.into_inner());
        let mut names: HashMap<(String, u32), String> = HashMap::new();
        {
            let mut q = conn.prepare(
                "SELECT tbl_name, idx, en_us FROM localized_texts WHERE tbl_column_name = 'name' \
                 AND tbl_name IN ('npcs', 'doodad_almighties', 'items', 'quest_contexts') AND en_us IS NOT NULL AND en_us != ''",
            )?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32, r.get::<_, String>(2)?)))?;
            for row in rows.flatten() {
                names.insert((row.0, row.1), row.2);
            }
        }
        for table in Table::ALL {
            let sql = match table {
                Table::Npcs => "SELECT id, name, COALESCE(model_id, 0), '' FROM npcs ORDER BY id",
                Table::Doodads => "SELECT id, name, 0, COALESCE(model, '') FROM doodad_almighties ORDER BY id",
                Table::Items => "SELECT id, name, 0, '' FROM items ORDER BY id",
                Table::Quests => "SELECT id, name, 0, '' FROM quest_contexts ORDER BY id",
            };
            let mut q = conn.prepare(sql)?;
            let rows = q.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u32,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, i64>(2)? as u32,
                    r.get::<_, String>(3)?,
                ))
            })?;
            let mut list = Vec::new();
            for (id, own, model_id, model) in rows.flatten() {
                let name = names.get(&(table.sql().to_string(), id)).cloned().unwrap_or(own);
                let search = format!("{id} {}", name.to_lowercase());
                list.push(Entry { id, name, model_id, model, search });
            }
            self.by_id.insert(table, list.iter().enumerate().map(|(i, e)| (e.id, i)).collect());
            self.lists.insert(table, list);
        }
        info!(
            "game data {}: {} npcs, {} doodads, {} items, {} quests ({} ms)",
            self.path,
            self.lists[&Table::Npcs].len(),
            self.lists[&Table::Doodads].len(),
            self.lists[&Table::Items].len(),
            self.lists[&Table::Quests].len(),
            started.elapsed().as_millis()
        );
        Ok(())
    }

    pub fn entry(&self, table: Table, id: u32) -> Option<&Entry> {
        self.by_id.get(&table)?.get(&id).and_then(|i| self.lists.get(&table)?.get(*i))
    }

    pub fn name(&self, table: Table, id: u32) -> Option<String> {
        self.entry(table, id).map(|e| e.name.clone())
    }

    /// The model reference to preview a doodad with (`cgf://` stripped; `prefab://...` kept).
    pub fn doodad_model(&self, id: u32) -> Option<String> {
        let m = &self.entry(Table::Doodads, id)?.model;
        if m.is_empty() {
            return None;
        }
        Some(match m.strip_prefix("cgf://") {
            Some(p) => norm(p),
            None => m.clone(),
        })
    }

    /// The model file of an NPC: an actor model (`.cdf` / `.chr`), or a `prefab://` element.
    pub fn npc_model(&mut self, npc_id: u32) -> Option<String> {
        let model_id = self.entry(Table::Npcs, npc_id)?.model_id;
        if let Some(f) = self.actor_files.get(&model_id) {
            return f.clone();
        }
        let conn = self.conn.get_mut().unwrap_or_else(|e| e.into_inner());
        let file: Option<String> = conn
            .query_row(
                "SELECT a.model_file FROM models m JOIN actor_models a ON a.id = m.sub_id AND m.sub_type = 'ActorModel' WHERE m.id = ?1",
                params![model_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .map(|f| norm(&f))
            .filter(|f| !f.is_empty())
            .or_else(|| {
                conn.query_row(
                        "SELECT e.file_path FROM models m JOIN prefab_elements e ON e.prefab_model_id = m.sub_id \
                         AND m.sub_type = 'PrefabModel' WHERE m.id = ?1 ORDER BY e.state_id LIMIT 1",
                        params![model_id],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten()
                    .filter(|p| p.starts_with("prefab://"))
            });
        self.actor_files.insert(model_id, file.clone());
        file
    }

    /// What the server sends for NPC `npc_id`: equipment slots and face / hair / skin look.
    pub fn npc_look(&self, npc_id: u32) -> Option<crate::npc_look::NpcLook> {
        crate::npc_look::load(&self.lock(), npc_id)
    }

    /// The weapons NPC look `npc` carries at peace.
    pub fn npc_carried(&self, npc: &crate::npc_look::NpcLook) -> Vec<crate::npc_look::Carried> {
        crate::npc_look::carried(&self.lock(), npc)
    }

    /// `npcs.scale` (1 when unset).
    pub fn npc_scale(&self, npc_id: u32) -> f32 {
        self.lock()
            .query_row("SELECT scale FROM npcs WHERE id = ?1", params![npc_id], |r| r.get::<_, Option<f64>>(0))
            .ok()
            .flatten()
            .map(|s| s as f32)
            .filter(|s| *s > 0.01)
            .unwrap_or(1.0)
    }

    /// All columns of row `id`, as text.
    pub fn row(&self, table: Table, id: u32) -> Result<Vec<Field>> {
        let conn = self.lock();
        let mut q = conn.prepare(&format!("SELECT * FROM {} WHERE id = ?1", table.sql()))?;
        let columns: Vec<String> = q.column_names().iter().map(|c| c.to_string()).collect();
        let values: Vec<SqlValue> = q.query_row(params![id], |r| (0..columns.len()).map(|i| r.get::<_, SqlValue>(i)).collect())?;
        Ok(columns
            .into_iter()
            .zip(values)
            .map(|(column, v)| {
                let (text, kind) = match v {
                    SqlValue::Null => (String::new(), ValueKind::Null),
                    SqlValue::Integer(i) => (i.to_string(), ValueKind::Integer),
                    SqlValue::Real(f) => (f.to_string(), ValueKind::Real),
                    SqlValue::Text(t) => (t, ValueKind::Text),
                    SqlValue::Blob(b) => (format!("<{} bytes>", b.len()), ValueKind::Blob),
                };
                Field { column, original: text.clone(), text, null: kind == ValueKind::Null, kind }
            })
            .collect())
    }

    /// The localized text of `table.column` for `id`, in English.
    pub fn localized(&self, table: Table, column: &str, id: u32) -> Option<String> {
        self.lock()
            .query_row(
                "SELECT en_us FROM localized_texts WHERE tbl_name = ?1 AND tbl_column_name = ?2 AND idx = ?3",
                params![table.sql(), column, id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
    }

    /// The FollowPath AI commands (`ai_commands.cmd_id = 2`), ordered by route and set.
    pub fn path_commands(&self) -> Vec<PathCommand> {
        read_path_commands(&self.lock())
    }

    /// Every AI command set: (id, name).
    pub fn command_sets(&self) -> Vec<(u32, String)> {
        let conn = self.lock();
        let Ok(mut st) = conn.prepare("SELECT id, COALESCE(name, '') FROM ai_command_sets ORDER BY id") else {
            return Vec::new();
        };
        st.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// Writes the FollowPath commands `now` over `before` (as [`read_path_commands`] gave them) in one transaction;
    /// returns the rows changed.
    pub fn save_path_commands(&mut self, before: &[PathCommand], now: &[PathCommand]) -> Result<usize> {
        self.check_writable()?;
        write_path_commands(self.conn.get_mut().unwrap_or_else(|e| e.into_inner()), before, now)
    }

    /// The housing rule tables ([`crate::housing`]).
    pub fn housing_rules(&self) -> Result<crate::housing::Rules> {
        Ok(crate::housing::read_rules(&self.lock())?)
    }

    pub fn housing_labels(&self) -> crate::housing::Labels {
        crate::housing::read_labels(&self.lock())
    }

    /// Writes the housing rule changes from `before` to `now` (one transaction); returns the rows written.
    pub fn save_housing_rules(&mut self, before: &crate::housing::Rules, now: &crate::housing::Rules) -> Result<usize> {
        self.check_writable()?;
        crate::housing::write_rules(self.conn.get_mut().unwrap_or_else(|e| e.into_inner()), before, now)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check_writable(&self) -> Result<()> {
        anyhow::ensure!(self.writable, "the database {} is open read-only", self.path);
        Ok(())
    }

    /// Writes the changed fields of row `id` and its English name / description texts, in one transaction.
    pub fn update_row(&mut self, table: Table, id: u32, fields: &[Field], texts: &[(String, String)]) -> Result<usize> {
        self.check_writable()?;
        let conn = self.conn.get_mut().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction()?;
        let mut changed = 0;
        // blobs are shown as a size only and never written back
        for f in fields.iter().filter(|f| f.text != f.original && f.column != "id" && f.kind != ValueKind::Blob) {
            let value = f.value();
            tx.execute(&format!("UPDATE {} SET \"{}\" = ?1 WHERE id = ?2", table.sql(), f.column.replace('"', "")), params![value, id])?;
            changed += 1;
        }
        for (column, text) in texts {
            let n = tx.execute(
                "UPDATE localized_texts SET en_us = ?1 WHERE tbl_name = ?2 AND tbl_column_name = ?3 AND idx = ?4",
                params![text, table.sql(), column, id],
            )?;
            if n == 0 {
                tx.execute(
                    "INSERT INTO localized_texts (id, tbl_name, tbl_column_name, idx, en_us) \
                     VALUES ((SELECT COALESCE(MAX(id), 0) + 1 FROM localized_texts), ?1, ?2, ?3, ?4)",
                    params![table.sql(), column, id, text],
                )?;
            }
            changed += 1;
        }
        tx.commit()?;
        self.actor_files.clear();
        self.reload_lists()?;
        Ok(changed)
    }

    /// Copies row `id` (and its localized texts) to a new id (`MAX(id) + 1`); returns the new id.
    pub fn clone_row(&mut self, table: Table, id: u32) -> Result<u32> {
        self.check_writable()?;
        let t = table.sql();
        let conn = self.conn.get_mut().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction()?;
        let new_id: i64 = tx.query_row(&format!("SELECT COALESCE(MAX(id), 0) + 1 FROM {t}"), [], |r| r.get(0))?;
        let columns: Vec<String> = {
            let q = tx.prepare(&format!("SELECT * FROM {t} LIMIT 0"))?;
            q.column_names().iter().map(|c| format!("\"{c}\"")).collect()
        };
        let rest: Vec<&String> = columns.iter().filter(|c| c.as_str() != "\"id\"").collect();
        let list = rest.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
        tx.execute(&format!("INSERT INTO {t} (id, {list}) SELECT ?1, {list} FROM {t} WHERE id = ?2"), params![new_id, id])?;
        tx.execute(
            "INSERT INTO localized_texts (id, tbl_name, tbl_column_name, idx, ko, en_us, zh_cn, ja, ru, zh_tw, de, fr) \
             SELECT (SELECT COALESCE(MAX(id), 0) FROM localized_texts) + ROW_NUMBER() OVER (), tbl_name, tbl_column_name, ?1, \
             ko, en_us, zh_cn, ja, ru, zh_tw, de, fr FROM localized_texts WHERE tbl_name = ?2 AND idx = ?3",
            params![new_id, t, id],
        )?;
        tx.commit()?;
        self.reload_lists()?;
        Ok(new_id as u32)
    }

    /// Deletes row `id` and its localized texts.
    pub fn delete_row(&mut self, table: Table, id: u32) -> Result<()> {
        self.check_writable()?;
        let conn = self.conn.get_mut().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction()?;
        tx.execute(&format!("DELETE FROM {} WHERE id = ?1", table.sql()), params![id])?;
        tx.execute("DELETE FROM localized_texts WHERE tbl_name = ?1 AND idx = ?2", params![table.sql(), id])?;
        tx.commit()?;
        self.reload_lists()?;
        Ok(())
    }

    /// The model of weapon item `id` (`item_weapons` -> `item_assets.path`: a `.cgf` or a `.cdf` carrier).
    pub fn item_weapon(&self, id: u32) -> Option<String> {
        self.lock()
            .query_row(
                "SELECT ia.path FROM item_weapons w JOIN item_assets ia ON ia.id = w.asset_id WHERE w.item_id = ?1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .map(|p| norm(&p))
            .filter(|p| p.contains('.'))
    }

    /// Whether item `id` is armour or clothing (`item_armors`).
    pub fn item_is_armor(&self, id: u32) -> bool {
        self.lock().query_row("SELECT 1 FROM item_armors WHERE item_id = ?1", params![id], |_| Ok(())).is_ok()
    }

    /// The pak path of item `id`'s icon (`items.icon_id` -> `icons.filename` under `ui/icon/`).
    pub fn item_icon(&self, id: u32) -> Option<String> {
        self.lock()
            .query_row(
                "SELECT ic.filename FROM items i JOIN icons ic ON ic.id = i.icon_id WHERE i.id = ?1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .filter(|f| !f.trim().is_empty())
            .map(|f| format!("ui/icon/{}", f.trim().to_ascii_lowercase()))
    }

    /// The NPC that stands for a quest, with its role: the one who gives it (`QuestActConAcceptNpc`), else the one it
    /// is turned in to (`QuestActConReportNpc`), else the first step that names one (`quest_components.npc_id`).
    pub fn quest_npc(&self, quest: u32) -> Option<(u32, &'static str)> {
        let conn = self.lock();
        for (detail, table, role) in [
            ("QuestActConAcceptNpc", "quest_act_con_accept_npcs", "gives the quest"),
            ("QuestActConReportNpc", "quest_act_con_report_npcs", "takes the quest in"),
        ] {
            let npc: Option<i64> = conn
                .query_row(
                    &format!(
                        "SELECT n.npc_id FROM quest_components c JOIN quest_acts a ON a.quest_component_id = c.id \
                         JOIN {table} n ON n.id = a.act_detail_id WHERE c.quest_context_id = ?1 AND a.act_detail_type = ?2 \
                         AND n.npc_id > 0 ORDER BY c.id LIMIT 1"
                    ),
                    params![quest, detail],
                    |r| r.get(0),
                )
                .ok();
            if let Some(npc) = npc {
                return Some((npc as u32, role));
            }
        }
        conn.query_row(
            "SELECT npc_id FROM quest_components WHERE quest_context_id = ?1 AND npc_id > 0 ORDER BY id LIMIT 1",
            params![quest],
            |r| r.get::<_, i64>(0),
        )
        .ok()
        .map(|n| (n as u32, "named by a step"))
    }

    /// The steps of a quest (`quest_components`): id, kind, NPC with name.
    pub fn quest_components(&self, quest: u32) -> Vec<(u32, u32, u32, String)> {
        let conn = self.lock();
        let Ok(mut q) = conn.prepare(
            "SELECT id, COALESCE(component_kind_id, 0), COALESCE(npc_id, 0) FROM quest_components WHERE quest_context_id = ?1 ORDER BY id",
        ) else {
            return Vec::new();
        };
        let rows = q.query_map(params![quest], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32, r.get::<_, i64>(2)? as u32)));
        rows.map(|rows| rows.flatten().map(|(id, kind, npc)| (id, kind, npc, self.name(Table::Npcs, npc).unwrap_or_default())).collect())
            .unwrap_or_default()
    }
}

/// Quest component kinds, for display (the server's `quests::model::kind`).
pub fn component_kind(kind: u32) -> &'static str {
    match kind {
        1 => "none",
        2 => "start",
        3 => "supply",
        4 => "progress",
        5 => "fail",
        6 => "ready",
        7 => "drop",
        8 => "reward",
        _ => "?",
    }
}

/// A FollowPath AI command (`ai_commands` row with `cmd_id = 2`): the NPCs of command set `set` walk `route`.
#[derive(Clone, Debug, PartialEq)]
pub struct PathCommand {
    /// Row id; 0 for a row not in the database yet.
    pub id: u32,
    pub set: u32,
    pub set_name: String,
    /// `param1` (1 for most rows, 2 for some), kept as is.
    pub param1: i64,
    /// `param2`: the route name.
    pub route: String,
}

pub fn read_path_commands(conn: &Connection) -> Vec<PathCommand> {
    let Ok(mut st) = conn.prepare(
        "SELECT c.id, c.cmd_set_id, COALESCE(s.name, ''), COALESCE(c.param1, 1), c.param2 FROM ai_commands c          LEFT JOIN ai_command_sets s ON s.id = c.cmd_set_id WHERE c.cmd_id = 2 AND TRIM(COALESCE(c.param2, '')) <> ''          ORDER BY c.param2, c.cmd_set_id, c.id",
    ) else {
        return Vec::new();
    };
    st.query_map([], |r| {
        Ok(PathCommand {
            id: r.get::<_, i64>(0)? as u32,
            set: r.get::<_, i64>(1)? as u32,
            set_name: r.get(2)?,
            param1: r.get(3)?,
            route: r.get::<_, String>(4)?.trim().to_string(),
        })
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

/// Deletes the rows of `before` missing from `now`, updates the changed ones and inserts the new ones (id 0, numbered
/// after the table's last id, so they run after the set's other commands).
pub fn write_path_commands(conn: &mut Connection, before: &[PathCommand], now: &[PathCommand]) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut changed = 0;
    for b in before.iter().filter(|b| !now.iter().any(|n| n.id == b.id)) {
        changed += tx.execute("DELETE FROM ai_commands WHERE id = ?1 AND cmd_id = 2", params![b.id])?;
    }
    for n in now {
        if n.id == 0 {
            tx.execute(
                "INSERT INTO ai_commands (id, cmd_set_id, cmd_id, param1, param2)                  VALUES ((SELECT COALESCE(MAX(id), 0) + 1 FROM ai_commands), ?1, 2, ?2, ?3)",
                params![n.set, n.param1, n.route],
            )?;
            changed += 1;
        } else if before.iter().any(|b| b.id == n.id && b != n) {
            changed += tx.execute(
                "UPDATE ai_commands SET cmd_set_id = ?1, param1 = ?2, param2 = ?3 WHERE id = ?4 AND cmd_id = 2",
                params![n.set, n.param1, n.route, n.id],
            )?;
        }
    }
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARED: &str = "D:/aaemu-live/rs-game/Data/compact.sqlite3";

    #[test]
    fn lists_names_and_models_from_the_game_db() {
        if !Path::new(SHARED).exists() {
            return;
        }
        let mut db = GameData::open(Path::new(SHARED), true).unwrap();
        assert!(db.lists[&Table::Npcs].len() > 10_000);
        assert_eq!(db.name(Table::Npcs, 502).as_deref(), Some("Temple Priestess"));
        assert_eq!(db.npc_model(502).as_deref(), Some("objects/characters/nuian/female/nude/nu_f.cdf"));
        assert_eq!(
            db.doodad_model(1373).as_deref(),
            Some("objects/env/01_nuia/001_ndeco/quest/ndeco_quest_nu_m_deadbody.cgf")
        );
        let row = db.row(Table::Items, db.lists[&Table::Items][0].id).unwrap();
        assert!(row.iter().any(|f| f.column == "price"));
        assert!(db.update_row(Table::Items, 1, &[], &[]).is_err(), "read-only refuses writes");
        let commands = db.path_commands();
        assert!(commands.len() > 200 && commands.iter().all(|c| c.id > 0 && !c.route.is_empty()));
        assert!(db.save_path_commands(&commands, &[]).is_err(), "read-only refuses command writes");
    }

    #[test]
    fn edits_clone_and_delete_on_a_copy() {
        if !Path::new(SHARED).exists() {
            return;
        }
        let copy = std::env::temp_dir().join(format!("aa-editor-db-{}.sqlite3", std::process::id()));
        std::fs::copy(SHARED, &copy).unwrap();
        {
            let mut db = GameData::open(&copy, false).unwrap();
            let id = db.lists[&Table::Items][10].id;
            let mut fields = db.row(Table::Items, id).unwrap();
            let price = fields.iter_mut().find(|f| f.column == "price").unwrap();
            price.text = "12345".into();
            db.update_row(Table::Items, id, &fields, &[("name".into(), "Editor Test Sword".into())]).unwrap();
            assert_eq!(db.name(Table::Items, id).as_deref(), Some("Editor Test Sword"));
            assert_eq!(db.row(Table::Items, id).unwrap().iter().find(|f| f.column == "price").unwrap().text, "12345");
            let new_id = db.clone_row(Table::Items, id).unwrap();
            assert_eq!(db.name(Table::Items, new_id).as_deref(), Some("Editor Test Sword"));
            db.delete_row(Table::Items, new_id).unwrap();
            assert!(db.entry(Table::Items, new_id).is_none());
        }
        let _ = std::fs::remove_file(&copy);
    }

    #[test]
    fn path_commands_are_repointed_added_and_removed() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_commands (id INTEGER, cmd_set_id INTEGER, cmd_id INTEGER, param1 INTEGER, param2 TEXT);
             CREATE TABLE ai_command_sets (id INTEGER, name TEXT, can_interact TEXT);
             INSERT INTO ai_command_sets VALUES (76, 'kron', 'f'), (82, 'wish', 'f');
             INSERT INTO ai_commands VALUES (1058, 76, 2, 1, 'long_quest_kron_01'), (1059, 76, 4, 1, '0'),
                 (1061, 76, 2, 1, 'long_quest_kron_01'), (1186, 82, 2, 2, 'AIPath_140_wish_49');",
        )
        .unwrap();
        let before = read_path_commands(&conn);
        assert_eq!(before.len(), 3, "only cmd_id 2 rows");
        assert_eq!((before[0].id, before[0].route.as_str(), before[0].set_name.as_str()), (1186, "AIPath_140_wish_49", "wish"));
        let mut now = before.clone();
        now[0].route = "new_route".into(); // re-point
        now.retain(|c| c.id != 1061); // remove
        now.push(PathCommand { id: 0, set: 82, set_name: "wish".into(), param1: 1, route: "added".into() });
        assert_eq!(write_path_commands(&mut conn, &before, &now).unwrap(), 3);
        let after = read_path_commands(&conn);
        let routes: Vec<(u32, u32, &str)> = after.iter().map(|c| (c.id, c.set, c.route.as_str())).collect();
        assert_eq!(routes, vec![(1187, 82, "added"), (1058, 76, "long_quest_kron_01"), (1186, 82, "new_route")]);
        let other: i64 = conn.query_row("SELECT COUNT(*) FROM ai_commands WHERE cmd_id <> 2", [], |r| r.get(0)).unwrap();
        assert_eq!(other, 1, "other commands untouched");
        assert_eq!(write_path_commands(&mut conn, &after, &after).unwrap(), 0);
    }
}
