# AA Editor: a world editor for ArcheAge 1.2

A world editor for **ArcheAge 1.2**, written in Rust on [Bevy](https://bevyengine.org) 0.20 and
[egui](https://github.com/emilk/egui). It renders the real game world from the original client files, using the streamed
terrain, objects, vegetation, water and sky from [AABevy1.2](https://github.com/NickMesser/AABevy1.2) (a client remake).
On top of that it gives you tools for:

* NPC and doodad placement
* terrain sculpting, painting and holes
* static world objects
* the game database (NPCs, doodads, items and quests)
* a play mode to walk around your edits

It saves to the files an [AAEmu](https://github.com/AAEmu/AAEmu) server reads, and it can export terrain into a real
`game_pak`.

> Work in progress. This is a fan project, not affiliated with XL Games or Kakao Games. **No game files are included.**
> You need your own ArcheAge 1.2 client and an AAEmu server's data.

![The editor over Nuia's starting village](docs/screenshots/overview.jpg)

## Screenshots

| | |
|---|---|
| ![NPC selection](docs/screenshots/select_npc.jpg) **Select:** NPC 502 Temple Priestess, dressed from her equip packs, with the gizmo and inspector | ![World objects](docs/screenshots/objects.jpg) **Objects:** a `brush.dat` house selected (yellow box); vegetation and entity markers |
| ![Terrain](docs/screenshots/terrain_paint.jpg) **Terrain:** paint brush with the cell's surface types; a hole cut into the ground | ![Play](docs/screenshots/play.jpg) **Play (F5):** walk the edited world with a character |
| ![Item row editor](docs/screenshots/item_editor.jpg) **Item editor:** every column of an `items` row, plus the English name and description | ![Quest row editor](docs/screenshots/quest_editor.jpg) **Quest editor:** a quest row and its `quest_components` steps |

Terrain hole, before and after (it is saved into the pak overlay and reloads on restart):

| Before | After |
|---|---|
| ![Before](docs/screenshots/hole_before.jpg) | ![After](docs/screenshots/hole_after.jpg) |

## What is supported

| Area | What you can do | Saves to |
|---|---|---|
| **NPC / doodad spawns** | Browse every NPC and doodad by English name and search them. Place one by clicking the ground. Select, drag (snaps to ground), rotate, duplicate and delete, with an inspector for UnitId, position, yaw / pitch / roll and scale. Undo / redo. | The AAEmu spawn JSON files (`npc_spawns*.json`, `doodad_spawns*.json`). Untouched entries keep their exact text and comments, new spawns go to `*_spawns_editor.json`, and the first overwrite keeps a `.editor-orig` backup. |
| **Previews** | Doodads and NPCs show their real models. Humanoid NPCs are dressed the way the server dresses them: armour from `equip_pack_cloths`, carried weapons from `equip_pack_weapons`, face and hair from `item_body_parts`, look from `total_character_customs`. Creatures get their skin attachments and an idle animation. | – |
| **Game database** | Row editor for NPCs, doodads, items and quests: every column, plus the English name and description. Changed fields are highlighted. Save, revert, clone as a new row (`MAX(id)+1`, texts copied) or delete. Quests list their steps, and "go to" flies to the step's NPC. | AAEmu's `compact.sqlite3`, one transaction per action. `--read-only-db` locks it. |
| **Terrain** | Sculpt brushes (raise, lower, smooth, flatten), each with a radius and strength. **Paint** surface types from the cell's own list, with thumbnails. **Hole** cuts or fills holes. **Tint** blends the baked cover colour, and Alt+click picks a colour. Undo per stroke. | `heightmap.dat` and `cover.ctc` per edited cell, in a pak overlay folder that the editor and the remade client both read (`AA_PAK_OVERLAY`). |
| **World objects** | Pick static brushes, vegetation and entity props (`brush.dat`, `object.dat`, `vegetation.dat`, `entities.xml`). Move, rotate, raise / lower, scale, copy and delete them. Objects follow terrain you sculpt under them. | The same binary formats in the pak overlay. Parse → write is byte-identical for every main_world cell. |
| **Pak export** | Writes the overlay into a real `game_pak` that the original client reads. It patches in place like the original patcher (same slot, a free block or appended), rewrites the MD5 / table / header, and writes an undo file. "Undo last export" restores the pak byte for byte. | A `game_pak` you choose. Use a **copy** of your client: the original client folder is refused. |
| **Server terrain** | Exports edited cells as a client-file overlay that the AAEmu Rust server reads for heights, and adds it to the server's `ClientData.Sources`. | `<server-data>/ClientOverlay/...` plus `Config.Local.json` (backup kept) |
| **Play (F5)** | A Nuian character walks the edited terrain: WASD, Shift to run, Space to jump, right mouse to turn. They fall through holes. | – |
| **Play on server** | Shows the server's port status and can restart a local AAEmu lane (asks twice). "Save all and launch client" starts the remade client against it with your overlay, and can `/move` you to the camera position (GM account). | – |

### Not supported yet

* The server's navmesh (`paths/*.bai`) is not regenerated. With `GeoDataMode` on, NPCs over edited ground still use the
  old heights.
* Painting can only use surface types the cell already lists. Adding a new layer needs `leveldata.xml` editing.
* Decals, roads, water volumes and interior (`visareas.dat`) objects are not editable. You can't add brand-new world
  objects from a model browser yet, only copies of existing ones.
* In-editor Play has no server: no NPC AI or quests. Use "Play on server" for that.
* Pak export is verified by reading the pak back. It has not been tested end to end on a full original-client copy.
* Only `main_world` has been exercised. Windows is the only platform tested.

## Requirements

* Rust **1.85+** (edition 2024). It was built with 1.99 on Windows 11.
* An **ArcheAge 1.2 client** (`game_pak`). It is only read.
* AAEmu server data:
  * the game database `compact.sqlite3`
  * the world spawn folder (`Data/Worlds/main_world` with `npc_spawns.json` / `doodad_spawns.json`)
* Optional: a running AAEmu Rust server and an `aa-client` build from AABevy1.2, for "Play on server".

The engine crates (`aa-pak`, `aa-cry`, `aa-anim`, `aa-world`, `aa-dev`) are git dependencies, pinned to a commit of
[AABevy1.2](https://github.com/NickMesser/AABevy1.2). The first build clones that repository (about 1 GB) and compiles
Bevy, which takes a while.

## Build and run

```sh
git clone https://github.com/NickMesser/aa-editor
cd aa-editor
cargo build -p aa-editor
```

The defaults point at the author's machine, so pass your own paths:

```sh
# where the client is (default D:/AA1.2/game_pak)
set AA_PAK=E:/ArcheAge/game_pak            # PowerShell: $env:AA_PAK = "E:/ArcheAge/game_pak"

cargo run -p aa-editor -- \
  --db        E:/aaemu/Data/compact.sqlite3 \
  --spawn-dir E:/aaemu/Data/Worlds/main_world \
  --pos 15578,15382                        # Nuia start (server coordinates)
```

To try it safely, point `--spawn-dir` at a **copy** of the spawn folder and add `--read-only-db`.

### Flags

| Flag | Meaning |
|---|---|
| `--world <name>` | world to load (default `main_world`) |
| `--pos x,y[,z]` | start camera position, in server / CryEngine coordinates |
| `--spawn-dir <dir>` | the `Data/Worlds/<world>` folder spawns are read from and saved to |
| `--mirror <dir>` | also copy saved spawn files here (repeatable), e.g. the server's runtime folder |
| `--db <file>` / `--read-only-db` | the game database to edit / never write it |
| `--overlay <dir>` | pak overlay folder for terrain and object saves (default `data/local/editor/overlay`) |
| `--mode select\|place\|terrain\|objects`, `--play` | start in that mode |
| `--select <unit_id>`, `--row <npcs\|doodads\|items\|quests>:<id>` | select a spawn / open a database row at start |
| `--export-pak <game_pak>` | export the overlay into that pak at startup |
| `--server-data <dir>`, `--server-config <file>` | the AAEmu server data folder / config for "Server terrain" |
| `--client-exe <file>`, `--lane-dir <dir>` | the remade client and the server lane for "Play on server" |
| `--shot <png> [--shot-after s]`, `--size WxH` | take a screenshot and exit (how the images above were made) |

Environment: `AA_PAK` (client pak), `AA_PAK_OVERLAY` (set by the editor to `--overlay`; set it for `aa-client` to play
on your terrain), `AA_PAK_CACHE_MB`.

### Controls

| Key | Action |
|---|---|
| `1` `2` `3` `4` | Select / Place / Terrain / Objects mode |
| Right mouse drag | look around (the wheel changes fly speed while held) |
| WASD, Q / E (or Space) | fly the camera (Shift is 8× faster) |
| Left drag | move the selection over the ground, or paint with a brush |
| `[` `]`, Shift + wheel | turn by 15° / 5° |
| PageUp / PageDown, `+` / `-` | raise / lower, scale (objects) |
| Del, Ctrl+D, F | delete, duplicate, focus |
| Ctrl+Z / Ctrl+Y | undo / redo |
| Ctrl+S | save everything |
| F5 / Esc | play / stop |

The server reads spawns and the database only at startup, so **restart your AAEmu game server** after saving them.
Terrain edits only need the client (through the overlay or a pak export).

## Tests

```sh
cargo test -p aa-editor
```

27 tests cover the JSON-with-comments rewriter, the spawn store, the heightmap / cover writers, the pak writer and its
undo, the server config edit and NPC dressing. Tests that need the real client or database return early when the
data isn't there.

## Documentation

[docs/aa-editor.md](docs/aa-editor.md) covers file formats, save locations, the pak export layout, server terrain, the
navmesh caveat, world-object editing internals and every check flag.

## License

MIT OR Apache-2.0, as declared in `Cargo.toml`. ArcheAge and its assets belong to their owners. None are included here.
