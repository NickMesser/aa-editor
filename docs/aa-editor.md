# aa-editor: world editor

`crates/aa-editor` is a world editor built on the client crates: the real streamed world of `aa-world` (terrain,
objects, vegetation, water, sky) with editing tools and egui panels on top.

```text
cargo run -p aa-editor                                   # main_world, Nuia start, the bevy lane's data
cargo run -p aa-editor -- --pos 15578,15382 --select 502 --shot shots/aa-editor/select.png --shot-after 20
cargo test -p aa-editor
```


## What it edits and where it saves

| What | Read from | Saved to |
|---|---|---|
| NPC / doodad spawns | every `npc_spawns*.json` / `doodad_spawns*.json` in `--spawn-dir` (default: the bevy lane's source `D:/aaemu-wt/bevy/rust/crates/aaemu-game/app/Data/Worlds/<world>`) | the same files. Moved and deleted spawns rewrite their file. Untouched entries keep their exact text and comments. New spawns go to `*_spawns_editor.json`, and the server loads that file as well. The first overwrite of a file keeps `<file>.json.editor-orig`. Saved files are also copied to each `--mirror`, by default the lane's runtime folder `D:/aaemu-live-bevy/rs-game/Data/Worlds/<world>`. |
| Game data: NPC, doodad, item and quest rows and their English names | `--db`, default the lane's copy `D:/aaemu-live-bevy/rs-game/Data/compact.sqlite3`. It falls back to the shared copy, read-only. | the same database: UPDATE, clone (`MAX(id)+1`, texts copied) and delete in one transaction each |
| Terrain heights, surface types, holes; cover colour | the pak (plus the overlay) | `heightmap.dat` (and `cover.ctc` when tinted) per edited cell in the pak overlay `--overlay`, default `data/local/editor/overlay/worlds/<world>/cells/XXX_YYY/client/terrain/` |
| Static world objects (brushes, vegetation, entity props) | the cell's `brush.dat`, `object.dat`, `vegetation.dat`, `vegetation/SX_SY.dat`, `entities.xml` (pak plus overlay) | the edited files of each cell in the pak overlay, `<overlay>/worlds/<world>/cells/XXX_YYY/client/`, same binary formats (see "World objects") |

The server loads spawns and game data once, at startup. Restart the lane's game server after saving.

**Pak overlay.** `aa-pak` now reads `AA_PAK_OVERLAY=<dir>`: a loose file at `<dir>/<pak path>` replaces or adds a pak
file for `read` and `exists`. The editor sets the overlay to its own folder, so saved terrain shows up when the
editor restarts. Run the remade client with the same variable to play on the edited terrain. The original
`game_pak` in `D:\AA1.2` is never written: that folder is read-only for this project.

**Pak export** (window "Pak export", or `--export-pak <game_pak>` at startup). It writes every overlay file into a
real `game_pak` that the original client reads. `aa_pak::write` (layout in its module docs) patches the pak in
place, the way the original patcher does:

* An entry with the same name is overwritten in its own slot when the new data fits in `size + padding`.
* Otherwise the data goes into a free block (`__unused__`) or is appended after the data. The old slot becomes a
  free block.
* The table and header are written again, with the MD5, sizes and FILETIMEs.

Overlay paths map to lowercase pak names under `game/`, for example
`game/worlds/main_world/cells/015_015/client/terrain/heightmap.dat`. A name the pak already has without `game/` (`bin32/...`) keeps that form. Before each export, an
undo file `<pak>.undo-<unix time>` is written next to the pak. It holds the old table and header (about 73 MB for the
full 1.2 pak) and the old bytes of every overwritten slot. "Undo last export" (or `aa_pak::write::undo`) restores
the pak byte for byte. To undo several exports, apply their undo files newest first.

The window refuses `D:/AA1.2` and anything under it. To play the edits in the original client:

1. Copy the whole client folder (about 25 GB) to a drive with space, e.g. `C:/AA1.2-edited`.
2. Save terrain in the editor. Then export with the target `C:/AA1.2-edited/game_pak`.
3. Start the client from the copy.

The export is checked by reading the pak back. For the server see "Server terrain" below.

## Server terrain

The Rust server has no height files of its own. `WorldHeightmap::load_cell_height_map`
(`aaemu-game/src/systems/npcs/heightmap.rs:406`) reads the client's
`game/worlds/<world>/cells/XXX_YYY/client/terrain/heightmap.dat` on demand through `ClientFileManager`
(`data/client_files.rs:393,474`). It tries the sources of `ClientData.Sources` in order and the first one that has the
file wins. A source is a `game_pak` or a plain folder laid out like the client. The lane's `Config.Local.json` lists only
`D:/AA1.2/game_pak`. Heights feed movement validation, NPC spawn / pathing heights and `GetHeight`. The reader quantises
to 5 cm * step (`NodeCell::init`), so server heights of an edited cell differ from the editor's by under 0.6 m (usually
much less), like the original data.

The window **Server terrain** (Terrain mode) has the button "Export terrain to server". It writes every edited cell with
the same writer as the overlay into `<server-data>/ClientOverlay/game/worlds/<world>/cells/XXX_YYY/client/terrain/heightmap.dat`
(an existing file first gets `.editor-orig`) and, if ticked, inserts `<server-data>/ClientOverlay` as the first entry of
`ClientData.Sources` in `<server-data>/../Config.Local.json` (backup `Config.Local.json.editor-orig`). Flags:
`--server-data <dir>` (default `D:/aaemu-live-bevy/rs-game/Data`), `--server-config <file>`, and for scripted runs
`--export-server-terrain` (exports once after `--sculpt ... --sculpt-save`). Unedited cells are not touched, so the server
keeps reading them from the pak.

**Restart the Rust game server** afterwards (config and cells are read at start; cells are cached once loaded).
To undo, delete the `ClientOverlay` folder (and restore `Config.Local.json.editor-orig`).

**Navmesh.** `.bai` files (`game/worlds/<world>/paths/XXX_YYY/*.bai`) are baked by the CryEngine tools and the server
only reads them (`systems/ai/geodata/manager.rs:186,431`); no generator exists. With `GeoDataMode: true`
(`Configurations/World.json`, on in the lane) `AiGeoData::get_height` returns the navmesh node height first and falls
back to the heightmap only where no navmesh node exists. So over edited ground that has navmesh the server still uses
the old Z for NPCs and A* paths. Either accept that, or set `GeoDataMode` to `false` (NPCs then use the heightmap and
lose A* pathing).

## Tools

* **Select (1).** Click a marker or model. Drag the selection over the ground to move it (it snaps to the ground; the
  View panel can turn snapping off). `[` / `]` turn it 15°, Shift + wheel turns it 5°. Del deletes, Ctrl+D duplicates,
  F focuses. The inspector edits UnitId, position, yaw / pitch / roll and doodad scale.
* **Place (2).** Pick an NPC or doodad in the browser, then click the ground. The new spawn faces the camera.
  Shift-click keeps placing.
* **Terrain (3).** Raise, lower, smooth and flatten brushes with a radius and strength. Hold the left button.
  Ctrl+Z undoes a stroke. Edits go into the streamed terrain patches (`TerrainPatch::set_heights` plus a rebuilt
  mesh), so they draw with the real terrain materials and LODs. See "Terrain painting and holes" for the other brushes.

### Terrain painting and holes

* **Paint.** The panel lists the surface types of the cell under the cursor (its `leveldata.xml` `<SurfaceTypes>`,
  with the layer's diffuse map as a thumbnail). The brush writes the five surface bits of the samples whose 2 m
  square is within the radius. A slot only means something inside its cell, so the brush paints a *material*: each
  cell under it gets the slot naming that `DetailMaterial` (`game/` prefix and case ignored); cells without it keep
  theirs. Holes are never painted over. The cell's `R8Uint` surface map (owned by its `TerrainMaterial`) is
  overwritten in place and the material touched, so no material, mesh or array is rebuilt.
* **Hole.** Sets samples to 31 (Shift or "fill holes" fills them again with the pak's surface, else a neighbour's).
  The sector's `hasHoles` gate follows (cut: on; fill: off when no hole is left), the patch meshes of the touched
  sectors are rebuilt (`build_patch_mesh` cuts the quads). `Heights::raycast` passes through holes,
  `Heights::ground_at` is `None` over them, and Play falls in (it stops 30 m down: the editor has no cave collision).
* **Tint.** Blends the baked cover colour (`cover.ctc`) toward a colour; Alt+click picks the ground's colour. The cover
  texels are an encoded chromaticity and brightness (see `aa-world` "terrain/cover.ctc"), `cover_paint::encode`
  converts. Every level's BC1 tiles under the brush are decoded, blended, re-encoded at the same file offset and
  copied into the cell's cover image mips. Missing fine tiles (coasts) stay missing; the client derives them.
* Undo (Ctrl+Z) is per stroke for all of them. The panel shows the cost of the last stroke frame: paint 0.3 ms,
  hole 0.3 ms (with the mesh rebuild), tint 7 ms for a 20 m brush (dev build, CPU side).
* **Formats written.** `heightmap.dat`: the original file as the template (`heightmap_write`); every leaf is a full
  33 x 33 node with its own `offset` / `range` and the edited surface bits (an unchanged sample keeps its original
  bits, including the far-edge copies the reader ignores); `hasHoles` of a leaf is the edited gate, a parent's the OR
  of its leaves (as in the original files). `cover.ctc`: the original bytes with the touched tiles re-encoded, same
  size and table.
* Checks: `--brush <raise|lower|smooth|flatten|paint|hole|tint>` selects the brush; `--paint x,y --paint-surface
  <slot>`, `--hole x,y` (`--hole-erase` fills), `--tint x,y --tint-colour r,g,b` apply one dab each at startup (any
  mode, also with `--play`); `--edit-save` saves them. `--sculpt x,y` holds the selected brush for 120 frames and logs
  the slowest frame.

```text
aa-editor --spawn-dir <copy> --read-only-db --overlay <dir> --pos 15520,15190,170 --mode terrain --brush-radius 10 \
  --paint 15500,15262 --paint-surface 8 --tint 15500,15262 --tint-colour 0.36,0.28,0.17 --hole 15540,15262 --edit-save
aa-editor ... --pos 15540,15250,140 --play        # the player falls into the hole (log: "play: falling through a hole")
```

Before / after: [`screenshots/hole_before.jpg`](screenshots/hole_before.jpg), [`screenshots/hole_after.jpg`](screenshots/hole_after.jpg).
* **Browser.** NPCs, doodads, items and quests with English names (`localized_texts`), filtered by search. ✎ or a click
  on an item or quest opens the **row editor**: every column, plus the English name and item description. Changed
  columns are highlighted. Save, revert, clone as new, delete (tick the box first). Quests list their
  `quest_components` steps, and "go to" flies to a spawn of the step's NPC.
* **Play (F5).** A default Nuian character walks the edited terrain: WASD, Shift to run, Space to jump, the right
  mouse button to turn the camera, the wheel to zoom. Esc or F5 returns to the editor camera.
* **Objects (4).** Static world objects, see "World objects".
* Undo / redo (Ctrl+Z / Ctrl+Y) covers spawn edits; in terrain mode Ctrl+Z undoes the last stroke, in Objects mode the
  object edits. Ctrl+S saves everything.

## World objects

`world_objects/` (Objects mode, key 4). Editable: **brushes** (`brush.dat`, or the `object.dat` octree in cells
without one), **vegetation** (`vegetation.dat`, the octree's own instances, the dense-forest `vegetation/SX_SY.dat`)
and **static entity props** (`entities.xml` GeomEntity / BasicEntity / AnimObject / sleeping RigidBodyEx with a unique
`EntityGuid`). Read-only: interior objects of `visareas.dat`, playing animated CGAs, decals, roads, water, voxels.

* **Pick.** A ray against the oriented boxes of the drawn mesh parts. `aa-world` puts
  `objects::edit::ObjectPart { cell, kind, index }` on every spawned part; `index` is into the cell's `CellObjects`.
  Hover shows white part boxes, the selection a yellow object box. The window "World object" shows model, cell, type,
  index, source record (e.g. `brush.dat #12 (+ object.dat copy)`) and X / Y / Z / yaw / scale (editable).
* **Edit.** Drag over the ground (snaps like spawns), `[` `]` turn 15° (Shift + wheel 5°; vegetation turns in
  1/256 steps), PageUp / PageDown raise / lower 0.25 m (Shift 2 m), `+` / `-` scale 5 %, Del, Ctrl+D (a copy 2 m east,
  in the same file), F focus, Ctrl+Z / Ctrl+Y. Objects stay in their cell's files even when dragged out of the cell.
* **Display.** An edit moves (or hides) the drawn parts at once. When no drag or stroke runs, the cell's edited copy
  goes to `ObjectState::replace_cell_objects`, which rebuilds the touched 64 m sectors hidden and swaps them in.
  Indices never shift: deleted objects stay in the lists (left out of the bins), copies are appended. A cell the
  streamer unloads and reloads gets the edited copy back.
* **Save** (Ctrl+S, `Request::SaveObjects`). Only the changed files of a cell are written, by
  `aa_world::placement_write`: `SectorFile` (brush.dat / vegetation.dat tables, sector files), `ObjectTree`
  (object.dat: tables verbatim, nodes with typed items, unknown bytes kept) and `entities_xml` (span edits of
  `Pos` / `Rotate` / `Scale`, remove or copy an `<Entity>`; parent-relative values for `ParentGUID` children).
  Parse -> write is byte-identical for every `brush.dat`, `vegetation.dat` and `object.dat` of main_world (test
  `all_main_world_cells_round_trip`). A moved record goes to the sector of its new position, with its box moved
  (from the loaded box, so repeated turns do not grow it). A `brush.dat` brush's copy in `object.dat` (matched by
  model and matrix) is edited with it.
* **Conform.** When terrain heights change (sculpt or terrain undo), objects standing on that ground follow it:
  Z += new − old height at their position. Vegetation within 3 m of the ground, brushes and entities within 1.5 m
  (brushes under 40 m wide). On by default ("Objects follow terrain sculpting", `--no-conform` turns it off); the
  button "Conform objects to terrain now" does it once from the heights the objects last matched.
* Checks: `--select-object x,y [--select-kind brush|vegetation|entity]` selects the nearest editable object of the
  cell and focuses it; after 6 s `--move-selected-object dx,dy,dz`, `--delete-selected-object`, `--objects-save`.

## Previews

`preview.rs` is simplified from aa-client's `ingame::{creature, doodad_model, housing::prefab}`, which are binary-only
there:

* Doodads: the `.cgf` in `doodad_almighties.model`, or every mesh part of its `prefab://` element.
* NPCs: the actor model of `npcs.model_id` (`models` → `actor_models.model_file`, else a prefab element).
  * A humanoid NPC is dressed like the server dresses it (`npc_look.rs`): armour from `equip_pack_cloths`, weapons
    from `equip_pack_weapons` (carried on their sheath bones), face / hair from `item_body_parts`, and the look from
    `total_character_customs` (a fixed pick per NPC id when the server would pick a random one).
  * A creature `.cdf` / `.chr` gets its skin attachments and an idle animation. Rigid bone attachments are not added.
* `npcs.scale` and the doodad's `Scale` apply.

Models load nearest first, within a 12 ms budget per frame. Spawns within the view radius (default 220 m) are shown;
the rest wait until the camera gets close.

## Coordinates

Spawn files use server / Cry space: metres, Z up, `Yaw` in degrees counter-clockwise from north (forward =
`(-sin, cos)`). This matches `aa-world`'s coordinates (`cry_to_bevy`), so positions need no conversion.

## Play on server

Menu bar "Play on server" (or `--play-server`) opens a window. Code: `crates/aa-editor/src/play_server.rs`.

* **Ports**: login 27237, game 27239, stream 27250 of the bevy lane, probed every 2 s with a loopback connect.
* **Restart lane server** (asks twice: "Click again to restart the bevy lane server"): runs the lane's documented
  `Scripts/live-stack.sh stop` and `start` from `D:/aaemu-wt/bevy` (`AAEMU_LANE=bevy`, Git bash), output into
  `data/local/editor/play_server_restart.log` (a file, not a pipe: the servers inherit it and a pipe would never close).
  `start` returns when the game server registered (about a minute); the window shows the elapsed time and the log tail.
  `sync` is deliberately not run: it copies the lane's source `app/Data` over the runtime folder and would overwrite the
  database the editor saved there. The editor's own save already mirrors the spawn files into the runtime folder.
  `stop` also ends the lane's real 1.2 client. The server reads spawns and the database at startup only, so restart after
  saving those; terrain edits only need the client (`AA_PAK_OVERLAY`).
* **Save all and launch client**: pushes `SaveSpawns` and `SaveTerrain`, waits until they are consumed, then starts
  `aa-client --enter-world --account A --login 127.0.0.1:27237 [--character C]` with `AA_PAK_OVERLAY` set to the overlay,
  output in `data/local/editor/play_server_client.log`. With "Move the character to the camera's ground position" on, it
  also types the GM command `/move X Y Z` through the client's scripted keyboard (`--hud-key enter 6 --hud-type "/move ..." 6.5
  --hud-key enter 7`; the account must be a GM). **Stop client** kills only the process the editor started.
* The client exe: pass `--client-exe <path to aa-client.exe>` (built from [AABevy1.2](https://github.com/NickMesser/AABevy1.2)
  with `cargo build -p aa-client`; it must include the pak overlay, i.e. be from 26 Oct 2026 or later).
* Flags: `--lane-dir`, `--lane-name`, `--lane-ports a,b,c`, `--play-account` (default `bevylive1`; `bevylive2` when
  another client holds it, the server lets one connection per account in), `--play-character`, `--play-password-file`
  (default: the client's shared test password file), `--client-arg "--shot a.png --shot-after 40"` (repeatable, split at spaces, passed to the client), `--dry-run` (the
  restart only logs its command), `--play-server-auto launch|restart` (press the button at start; `restart` needs `--dry-run`).

aa-client has no start-position flag that reaches the server (`--pos x,y[,z]` moves the local player only), hence `/move`.

## Gaps

* Decals, roads, water volumes and interior (`visareas.dat`) objects cannot be edited; no new objects from a model
  browser yet (only copies). The octree node boxes of `object.dat` are not refitted after a move.
* Painting cannot add a surface type a cell does not list (that needs its `leveldata.xml` and the layer library).
  Surface painting changes the detail layer only; the cover colour needs the Tint brush.
* The server's navmesh (`paths/*.bai`, baked into the pak) is not regenerated. With `GeoDataMode` on it still answers
  heights over navmesh-covered ground (see "Server terrain").
* Play (F5) runs inside the editor, without the server: no NPC AI, no quests. "Play on server" (below) runs the real client against the lane server for that.
