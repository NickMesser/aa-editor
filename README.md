# AA Editor: world and UI editors for ArcheAge 1.2

Two editors for **ArcheAge 1.2**, written in Rust on [Bevy](https://bevyengine.org) 0.20 and
[egui](https://github.com/emilk/egui), on top of the engine crates of [AABevy1.2](https://github.com/NickMesser/AABevy1.2)
(a client remake that reads the original game files).

**The world editor** (`aa-editor`) renders the real game world: streamed terrain, objects, vegetation, water and sky.
It gives you tools for:

* NPC and doodad placement
* terrain sculpting, painting (including new surface types) and holes
* static world objects: move, copy, delete, place new models, plus decals, roads, water volumes and interiors
* the game database (NPCs, doodads, items and quests)
* a 3D model preview of every NPC, doodad, item and quest giver
* a play mode to walk around your edits

It saves to the files an [AAEmu](https://github.com/AAEmu/AAEmu) server reads, including the server's navmesh heights,
and it can export its edits into a real `game_pak`.

**The UI editor** (`aa-ui-editor`) runs the client's original Lua UI: world and character select, character create,
and the in-game HUD and its windows. You can inspect any widget and see the script lines that created and placed it.
Change it live and write the change back into the script, or edit any script as Lua and compile it back to the
client's `.alb` bytecode.

> Work in progress. This is a fan project, not affiliated with XL Games or Kakao Games. **No game files are included.**
> You need your own ArcheAge 1.2 client and an AAEmu server's data.

![The world editor over Nuia's starting village](docs/screenshots/overview.jpg)

## Screenshots

### World editor

| | |
|---|---|
| ![NPC selection](docs/screenshots/select_npc.jpg) **Select:** NPC 502 Temple Priestess, dressed from her equip packs, with the gizmo and inspector | ![World objects](docs/screenshots/objects.jpg) **Objects:** a `brush.dat` house selected (yellow box); vegetation and entity markers |
| ![Terrain](docs/screenshots/terrain_paint.jpg) **Terrain:** paint brush with the cell's surface types; a hole cut into the ground | ![Play](docs/screenshots/play.jpg) **Play (F5):** walk the edited world with a character |
| ![Item row editor](docs/screenshots/item_editor.jpg) **Item editor:** every column of an `items` row, plus the English name and description | ![Quest row editor](docs/screenshots/quest_editor.jpg) **Quest editor:** a quest row and its `quest_components` steps |
| ![New object](docs/screenshots/new_object.jpg) **More objects:** a new model placed from the browser of all 10,055 pak models | ![Library](docs/screenshots/world_library.jpg) **Other worlds:** `--world instance_library` |
| ![Housing zone](docs/screenshots/housing_zone.jpg) **Housing:** zone 212 "terrorhousing1" selected, with its corners and rule set | ![Housing rules](docs/screenshots/housing_rules.jpg) **Housing rule sets:** conditions and allowed building categories |

Terrain hole, before and after (it is saved into the pak overlay and reloads on restart):

| Before | After |
|---|---|
| ![Before](docs/screenshots/hole_before.jpg) | ![After](docs/screenshots/hole_after.jpg) |

Two more edits, each shown after a restart:

* **Water volume:** a pond raised 3 m (top: before).
* **Interior brush:** a library bookshelf deleted from `visareas.dat` (left: before, middle: selected, right: deleted).

| Water volume | Interior brush |
|---|---|
| ![Water](docs/screenshots/water_move.jpg) | ![Interior](docs/screenshots/interior_delete.jpg) |

### Model preview

A turntable you can drag and zoom. It follows the browser row under the mouse, the open database row or the selected
spawn (`--preview items:18395`).

| NPC | Weapon | Armour on a chosen body | Quest giver | Creature | Prefab doodad |
|---|---|---|---|---|---|
| ![NPC](docs/screenshots/preview_npc.jpg) | ![Weapon](docs/screenshots/preview_weapon.jpg) | ![Armour](docs/screenshots/preview_armor.jpg) | ![Quest](docs/screenshots/preview_quest.jpg) | ![Creature](docs/screenshots/preview_creature.jpg) | ![Prefab](docs/screenshots/preview_prefab.jpg) |

### UI editor

| | |
|---|---|
| ![Inspector](docs/screenshots/ui_inspect.jpg) **Inspector:** the bag window and its creation chain (`window_view:220 → window:90 → sort_inventory:647 → sort_bag:472`), plus the lines that set its anchor, size and visibility | ![Script editor](docs/screenshots/ui_script.jpg) **Script editor:** `inventory/sort_bag` as Lua, opened at the line that anchors the bag |
| ![Character create](docs/screenshots/ui_create.jpg) **Character create:** a gender button picked in the widget tree, with its anchors, extent and script lines | |

## What is supported

### World editor (`aa-editor`)

| Area | What you can do | Saves to |
|---|---|---|
| **NPC / doodad spawns** | Browse every NPC and doodad by English name and search them. Place one by clicking the ground. Select, drag (snaps to ground), rotate, duplicate and delete, with an inspector for UnitId, position, yaw / pitch / roll and scale. Undo / redo. | The AAEmu spawn JSON files (`npc_spawns*.json`, `doodad_spawns*.json`). Untouched entries keep their exact text and comments. New spawns go to `*_spawns_editor.json`, and the first overwrite keeps a `.editor-orig` backup. |
| **Models in the world** | Doodads and NPCs show their real models. Humanoid NPCs are dressed the way the server dresses them: armour from `equip_pack_cloths`, carried weapons from `equip_pack_weapons`, face and hair from `item_body_parts`, look from `total_character_customs`. Creatures get their skin attachments and an idle animation. | – |
| **Model preview** | Turntable 3D view of any NPC (dressed, with weapons), doodad, weapon, armour worn on a chosen race and gender, item icon, a quest's NPC, or the selected world object (brush, tree, prop). | – |
| **AI paths** | Paths mode (5): the routes NPCs walk, drawn in the world with their direction and the NPCs that follow them. Drag points, Shift+click to add, delete; new, duplicate, rename, reverse, close loop, snap to ground, walk preview; per-point actions (speed, loop). Give any NPC a route (`FollowPath`), or start a new route at an NPC. Edit the AI command sets that use a route (re-point, add, remove; renames carry them along). Undo / redo. | The server's `Data/Path/*.path` (with `.editor-orig` backups, copied to the runtime folder) and the spawn files |
| **Housing zones** | Housing mode (6): every housing zone of the world drawn on the ground, coloured by rule set. Drag corners or whole zones, Shift+click to add corners, delete them; draw new zones, duplicate, rename, delete. Pick each zone's area row and rule set. Edit the rule sets: English name and description, houseless only, not if owning a category, can extend, tax delay, doodad, the allowed building categories; new (copy) and delete. Undo / redo. | The zones' `housing_area.xml` in the pak overlay and the server's `ClientOverlay` (byte-identical for untouched zones, `.editor-orig` backups); `housing_areas`, `housing_groups`, `housing_group_categories` in the database |
| **Game database** | Row editor for NPCs, doodads, items and quests: every column, plus the English name and description. Changed fields are highlighted. Save, revert, clone as a new row (`MAX(id)+1`, texts copied) or delete. Quests list their steps, and "go to" flies to the step's NPC. | AAEmu's `compact.sqlite3`, one transaction per action. `--read-only-db` locks it. |
| **Terrain** | Sculpt brushes: raise, lower, smooth and flatten, each with a radius and strength. **Paint** surface types from the cell's own list, with thumbnails. **Hole** cuts or fills holes. **Tint** blends the baked cover colour, and Alt+click picks a colour. Undo per stroke. | `heightmap.dat` and `cover.ctc` per edited cell, in a pak overlay folder that the editor and the remade client both read (`AA_PAK_OVERLAY`). |
| **New surface types** | Paint any material the world uses (317 in main_world) into a cell that doesn't list it yet; with "more materials", also the other worlds' and every `aa_terrain` material of the pak (the terrain reloads to draw these when the stroke ends). | That cell's `leveldata.xml` in the pak overlay |
| **World objects** | Pick static brushes, vegetation and entity props (`brush.dat`, `object.dat`, `vegetation.dat`, `entities.xml`). Move, rotate, raise / lower, scale, copy and delete them. Objects follow terrain you sculpt under them. | The same binary formats in the pak overlay. Parse → write is byte-identical for every main_world cell. |
| **More objects** | Place new brushes and vegetation from a browser of every `objects/**.cgf` in the pak. Decals, roads and water: move, copy into new ones, delete, and reshape with handles in the view (road nodes and edge points, water outline points); water redraws at once; roads and decals save and redraw when you let go (or with "Save and redraw" when that is off). Interior (`visareas.dat`) brushes: move and delete. | `brush.dat` / `statobjs.dat` / `object.dat` / `vegetation.dat` / `visareas.dat` in the pak overlay. Untouched bytes stay the same. |
| **Pak export** | Writes the overlay into a real `game_pak` that the original client reads. It patches in place like the original patcher (same slot, a free block, or appended), rewrites the MD5, table and header, and writes an undo file. "Undo last export" restores the pak byte for byte. | A `game_pak` you choose. Use a **copy** of your client: the original client folder is refused. |
| **Server terrain** | Exports edited cells as a client-file overlay that the AAEmu Rust server reads for heights, and adds it to the server's `ClientData.Sources`. Also updates the navmesh (`paths/*.bai`): it moves with the edited ground, links over new holes, sculpted walls and too steep slopes are blocked, links over smoothed cliffs reopen, and triangles under sculpted hills and valleys are split so NPC heights and paths follow them. | `<server-data>/ClientOverlay/...` plus `Config.Local.json` (backup kept). The navmesh files also go to the pak overlay, with `.editor-orig` backups. |
| **Worlds and navigation** | The world menu (🌐) lists every world in the pak and restarts the editor on the one you pick, asking first about unsaved changes. A gizmo in the top-right corner of the view orbits (drag), looks along an axis (click X / Y / Z), zooms and pans. **Teleport (M)** opens a relief map of the world to click on, plus a searchable list of zones, the server's respawn / recall / world gate points and start point, your bookmarks, NPCs and doodads by name, and an `x, y[, z]` box. View → Fog turns the time of day's fog off (`arche_mall_world` and `instance_hadir_farm` are authored with a dense one). | Bookmarks: `data/local/editor/bookmarks.json` |
| **Play (F5)** | A Nuian character walks the edited terrain: WASD, Shift to run, Space to jump, right mouse to turn. They fall through holes. | – |
| **Play on server** | Shows the server's port status and can restart a local AAEmu lane (it asks twice). "Save all and launch client" starts the remade client against it with your overlay, and can `/move` you to the camera position (GM account). | – |

### UI editor (`aa-ui-editor`)

| Area | What you can do |
|---|---|
| **Screens** | The client's real screens, built from the original scripts with a fake game API: world select, character select, character create (race, gender, step), the in-game HUD with any window open (bag, character, skills, quests, world map, options … or any `UIC_*` content), and the loading screen. Rebuild (F5) runs the scripts again. |
| **Widgets** | Widget tree with search. Pick on screen with F2 (or Alt+click): hover and selection outlines, anchor lines. Drag to move, drag the corner to resize. F1 hides the editor. |
| **Inspector** | Class, layer, parents, Lua paths (`bag.titleBar.closeButton`), rectangle, show / alpha, extent, anchors (point, target, relative point, offsets; add / remove), text, a drawable's texture, colour and coords, drawables and handlers. Also the script lines that **created** the object and that last **set each property**. |
| **Write into script** | Turns the live edits into source changes. It rewrites the literal arguments of the line that set the property, keeping the call's form. If the arguments are computed, it adds a call after that line, or after the creation line when nothing set the property. It shows a preview, then applies, saves and rebuilds. |
| **Script editor** | Browse every `.alb` and `toc.g`: all of them, this screen's in load order, or the edited ones. Lua highlighting, find / replace, go to line, compile check. **Save** compiles to the client's `.alb` layout into the pak overlay and rebuilds the screen. Revert to original. New scripts can be added to a `toc.g`. |
| **Console** | Load and runtime errors (click to open the line), `print` output, and a Lua prompt in the running UI. |
| **Export** | Writes the overlay's `scriptsbin/` and `ui/` files into a copy of `game_pak`, with an undo file. |

### Not supported yet

* The navmesh's forbidden areas (`areasmission`) aren't edited, and the new triangles and links haven't been walked by
  the server's NPCs yet (they were checked against the server's file rules).
* Textures that aren't in the pak can't be added as surface types.
* Roads and decals redraw by saving (no unsaved preview of their drawn shape). Decal size and interior area / portal shapes are
  not editable.
* In-editor Play has no server, so no NPC AI or quests. Use "Play on server" for that.
* Pak export is verified by reading the pak back. It has not been tested end to end on a full original-client copy.
* UI editor:
  * Compiled scripts load in the remade client's Lua but have not been tested in the original client.
  * The game data is fake, so windows the fakes don't drive (auction, mail, housing …) may be empty.
  * The login screen builds and can be edited through the tree and inspector, but `aa-ui` draws nothing for it yet.
  * Translucent panels blend slightly lighter than in game.
* Windows is the only platform tested.

## Requirements

* Rust **1.85+** (edition 2024). It was built with 1.99 on Windows 11.
* An **ArcheAge 1.2 client** (`game_pak`). It is only read.
* AAEmu server data:
  * the game database `compact.sqlite3`
  * the world spawn folder (`Data/Worlds/main_world` with `npc_spawns.json` / `doodad_spawns.json`)
* Optional: a running AAEmu Rust server and an `aa-client` build from AABevy1.2, for "Play on server".
* For the UI editor's Lua sources, either of:
  * the scripts decompiled with [unluac](https://sourceforge.net/projects/unluac/), laid out like `scriptsbin/`
    (`--decompiled <dir>`);
  * `--unluac <unluac.jar>` with Java, which decompiles each script when you open it.

  Without either, scripts open as a read-only disassembly.

The engine crates (`aa-pak`, `aa-cry`, `aa-anim`, `aa-world`, `aa-ui`, `aa-lua`, `aa-dev`) are git dependencies, pinned
to a commit of [AABevy1.2](https://github.com/NickMesser/AABevy1.2). The first build clones that repository (about 1 GB)
and compiles Bevy, which takes a while.

## Build and run

```sh
git clone https://github.com/NickMesser/aa-editor
cd aa-editor
cargo build -p aa-editor -p aa-ui-editor
```

The defaults point at the author's machine, so pass your own paths:

```sh
# where the client is (default D:/AA1.2/game_pak)
set AA_PAK=E:/ArcheAge/game_pak            # PowerShell: $env:AA_PAK = "E:/ArcheAge/game_pak"

cargo run -p aa-editor -- \
  --db        E:/aaemu/Data/compact.sqlite3 \
  --spawn-dir E:/aaemu/Data/Worlds/main_world \
  --pos 15578,15382                        # Nuia start (server coordinates)

cargo run -p aa-ui-editor -- --screen world --open toggle_bag --decompiled E:/aa-scripts/decompiled
cargo run -p aa-ui-editor -- --screen character_create --race elf --gender female --step 2
```

To try the world editor safely, point `--spawn-dir` at a **copy** of the spawn folder and add `--read-only-db`.

Both editors share the pak overlay (`data/local/editor/overlay`), so one Pak export writes world and UI edits together.
[docs/aa-ui-editor.md](docs/aa-ui-editor.md) lists the UI editor's flags.

### World editor flags

| Flag | Meaning |
|---|---|
| `--world <name>` | world to load (default `main_world`) |
| `--pos x,y[,z]` | start camera position, in server / CryEngine coordinates |
| `--look yaw,pitch` | start camera direction in degrees (yaw 0 = north, clockwise; pitch -90 = straight down) |
| `--no-fog` | start with the time of day's fog off (View → Fog), for worlds authored with a dense one |
| `--spawn-dir <dir>` | the `Data/Worlds/<world>` folder spawns are read from and saved to |
| `--mirror <dir>` | also copy saved spawn files here (repeatable), e.g. the server's runtime folder |
| `--db <file>` / `--read-only-db` | the game database to edit / never write it |
| `--overlay <dir>` | pak overlay folder for terrain and object saves (default `data/local/editor/overlay`) |
| `--mode select\|place\|terrain\|objects\|paths\|housing`, `--play` | start in that mode |
| `--select <unit_id>`, `--row <npcs\|doodads\|items\|quests>:<id>` | select a spawn / open a database row at start |
| `--preview <npcs\|doodads\|items\|quests>:<id>` | open the model preview on that row |
| `--teleport` | open the teleport window at start |
| `--mode paths`, `--path <name>`, `--path-dir <dir>` | start in Paths mode / on that route; the server's `Data/Path` folder |
| `--mode housing`, `--housing <area id>`, `--housing-tab rules` | start in Housing mode / on the zone of that `housing_areas` id / on the rule sets |
| `--preview-open` | open the model preview at start (it follows the selection) |
| `--add-surface`, `--more-objects` | open the "Add surface types" / "More objects" windows |
| `--export-pak <game_pak>` | export the overlay into that pak at startup |
| `--server-data <dir>`, `--server-config <file>` | the AAEmu server data folder / config for "Server terrain" |
| `--client-exe <file>`, `--lane-dir <dir>` | the remade client and the server lane for "Play on server" |
| `--shot <png> [--shot-after s]`, `--size WxH` | take a screenshot and exit (how the images above were made) |

Environment variables:

* `AA_PAK`: the client's pak.
* `AA_PAK_OVERLAY`: the editor sets it to `--overlay`. Set it for `aa-client` too, to play on your edits.
* `AA_PAK_CACHE_MB`.

### World editor controls

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
| M | teleport map |
| `5`, Shift+click, Del | Paths mode; add a route point; remove it |
| `6`, Shift+click, Del, Enter | Housing mode; add a zone corner; remove it; close a new zone |
| Gizmo (top right of the view) | drag to orbit, click an axis to look along it, drag 🔍 to zoom, ✋ to pan |
| F5 / Esc | play / stop |

The server reads spawns, AI paths, housing zones and the database only at startup, so **restart your AAEmu game server** after saving them.
Terrain and UI edits only need the client (through the overlay or a pak export).

### UI editor keys

F1 hides the panels, F2 toggles pick mode, Alt+click picks at any time, and F5 rebuilds the screen. In the script
editor: Ctrl+S saves (compile and rebuild), Ctrl+F finds, Ctrl+G goes to a line. In game: I / B / C / K / L / M / Esc
open the client's windows.

## Tests

```sh
cargo test -p aa-editor -p aa-ui-editor
```

* **World editor (75 tests):**
  * the JSON-with-comments rewriter and the spawn store
  * the heightmap, cover, navmesh and `leveldata.xml` writers
  * byte-identical round trips of the object files of several worlds
  * the pak writer and its undo
  * the server config edit and NPC dressing
  * the gizmo's axis views, the world-switch command line, the teleport places of main_world
  * housing zones: all 113 zone files of the pak regenerate byte for byte; new, moved and deleted zones; rule set edits written and read back
  * AI route files, `FollowPath` and AI command edits, navmesh walls, reopened links and split triangles, road / water reshaping, more surface materials
  * every world of the pak: placement files write back byte for byte, heightmaps within quantisation
* **UI editor (13 tests):**
  * the source patcher: every `AddAnchor` form, computed arguments, the creation-line fallback
  * `.alb` compilation: all 961 decompiled scripts compile and load back
  * highlighting and export

Tests that need the real client, database or decompiled scripts return early when the data isn't there.

## Documentation

* [docs/aa-editor.md](docs/aa-editor.md): file formats, save locations, the pak export layout, server terrain and
  navmesh, world-object editing internals, the model preview and every check flag.
* [docs/aa-ui-editor.md](docs/aa-ui-editor.md): how screens are built, where scripts come from, call-site tracking,
  writing changes back into scripts, compiling to `.alb`, and the flags.

## License

MIT OR Apache-2.0, as declared in `Cargo.toml`. ArcheAge and its assets belong to their owners. None are included here.
