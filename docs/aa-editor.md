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
| New objects; decals, roads, water volumes, interior brushes | the same files plus `statobjs.dat` and `visareas.dat` | the same overlay folder (see "More objects") |
| New surface types of a cell | the cell's `leveldata.xml` | `<overlay>/worlds/<world>/cells/XXX_YYY/client/leveldata.xml` (see "Adding surface types") |
| Server navmesh heights | `game/worlds/<world>/paths/XXX_YYY/*.bai` from the pak | with "Export terrain to server": `<server-data>/ClientOverlay/game/worlds/...` and `<overlay>/worlds/...` (see "Server terrain") |
| Housing zones (where players may build) and their rule sets | the zones' `game/worlds/<world>/level_design/zone/<key>/client/housing_area.xml` (pak plus overlay) and the database's `housing_areas`, `housing_groups`, `housing_group_categories` | the changed zone files in the pak overlay and `<server-data>/ClientOverlay/game/worlds/...` (first overwrite keeps `.editor-orig`; the overlay is registered in the server config), the rules in the database (see "Housing zones") |
| AI paths (the routes NPCs walk) and which NPC follows which | the server's `Data/Path/*.path` (`--path-dir`, default `Data/Path` next to the spawn folder's `Data/Worlds`) and the spawns' `FollowPath` | the same folder (first overwrite or removal keeps `<name>.path.editor-orig`), copied to the mirrors' `Path` folders; `FollowPath` is saved with the spawns (see "AI paths") |

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

**Navmesh.** `.bai` files (`game/worlds/<world>/paths/XXX_YYY/*.bai`, 256 m blocks, 4 x 4 per cell) are baked by the
CryEngine tools and the server only reads them (`systems/ai/geodata/manager.rs:186,431`, format in `geodata/bai.rs`).
With `GeoDataMode: true` (`Configurations/World.json`, on in the lane) `AiGeoData::get_height` answers from the
navmesh triangles / nodes first, so the editor moves the navmesh with the ground (`navmesh.rs`):

* After a successful "Export terrain to server", every `netmission*.bai` and `vertsmission*.bai` of the blocks of
  each edited cell is read from the **original** pak bytes (`Pak::read_packed`, never an earlier export) and patched in
  place. A point that sat on the original ground (within 1.5 m of the pak heightmap) gets `Z += new - old` ground
  height at its XY: netmission node positions, link edge centres and the block bbox Z; vertsmission vertices. Points
  off the ground (bridges, decks, caves) keep their Z. Shifts under 5 cm are ignored: the heightmap writer
  re-quantises every sector by about 1 cm.
* A link that touches a node over a **new** hole gets `max_pass_radius = -1` (blocked).
* Only changed files are written, lowercase, to `<server-data>/ClientOverlay/game/worlds/<world>/paths/XXX_YYY/` and to
  the pak overlay (so a pak export carries them), each with `.editor-orig` when a file is overwritten the first time.
  Everything except the patched Z / radius fields stays byte-identical (checked: 0 other bytes differ).
* **Walls and slopes.** A link between two nodes on the ground is blocked as well when the edited ground between
  them rises more than 1.5 m above the line between the nodes, or gets steeper than about 50° (rise over run 1.2),
  where the original ground did not. Sampled every metre along the link. So NPCs no longer path across walls and
  cliffs you sculpt; links over bridges (nodes off the ground) are left alone.
* **Reopened links.** The other way round: a blocked link between two nodes on the ground opens when the ground
  between them was a wall or too steep before the edit and can be walked now (a cliff smoothed into a ramp). Its
  `max_pass_radius` becomes half the shared edge's length, as the bake writes it (from the paired `vertsmission`).
  Links blocked on ground that was already walkable (obstacles, forbidden areas) stay blocked.
* **New triangles** (`navmesh_add.rs`). The bake triangulates *all* the ground, sea and cliffs included (a block of
  open sea is 6 triangles of 180 m); walkability is in the links, and the server's height is the plane of the
  enclosing triangle. A hill raised or a valley cut inside a triangle is invisible to that plane, so such triangles
  are split: when a point of a 4 m lattice inside a triangle (or every 4 m along one of its edges) has ground that
  moved more than 1.5 m and lies more than 1.5 m off the triangle's plane, the triangle is replaced by a constrained
  Delaunay triangulation (`spade`) of its corners and the lattice / edge points of the edited area plus a one-point
  ring, at the new ground height. The first child keeps the triangle's record and id, the others get the next ids;
  new corners are appended to the `vertsmission`. Children sharing an edge get a link each way (radius = half the
  edge, or -1 when the ground between their centres has a hole, a wall over 1.5 m or a slope over 1.2), with the
  bake's constant fields (exposure 1, `0, 0, 327.66`, bytes `1, 49`); the triangle's links to its neighbours move to
  the child on that edge. Two split neighbours share their edge points, and their children are linked where the old
  link was open. The bake repeats a triangle in every block file it reaches into (the server merges nodes by XY), so
  the split depends only on the triangle's world corners (to the millimetre) and the ground, and the export also
  runs over the ring of blocks around the edited cells: every copy splits the same way. Checked on a sculpted mound
  in the sea east of the Nuia start: 6 triangles split into 234 over 5 blocks, 0 disagreements between the files
  (the server's merge view), and test islands / hills / cliffs / shared edges (`navmesh_add` tests).
* **Not updated:** `areasmission*.bai` (forbidden areas, designer paths) and the `hidemission` files (the server
  doesn't load them).
* Status line: "Navmesh: N file(s), P point(s) moved, K off the ground kept, L link(s) blocked (W by new walls or
  slopes, the rest over holes), R reopened, S triangle(s) split into C". Tests `navmesh::tests`: untouched files stay
  byte-identical and most points lie on the ground (main_world and instance_prologue); a synthetic block with a bridge
  and a hole; a sculpted wall blocks a link, a raise of everything and an old wall do not; a smoothed wall reopens one.
  `navmesh_add::tests`: a hill inside a triangle and one on a shared edge (heights within 1.5 m, every node reachable),
  a cliff (links over it blocked), fitting triangles untouched, two copies of a triangle split alike, and an island
  raised in a real sea block.

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
* **Adding surface types** (`surface_add.rs`, window "Add surface types" next to the Paint panel, or
  `--add-surface`). Tick "add missing surface types" and pick any material of the world (317 in main_world: the
  distinct `DetailMaterial`s of all cells, each with the cell it comes from). Painting it into a cell that lacks it
  appends a copy of that `<SurfaceType .../>` to the cell's `<SurfaceTypes>` (new slot = the old count; a cell with 31
  is refused) and writes `leveldata.xml` to the overlay at once. The world's layer library already holds every
  material some cell uses, so no texture is added: the loaded cell's shader table gets the donor's entry for the new
  slot (uv offsets recomputed for this cell) and the material shows immediately; after a restart the library reads the
  overlay's `leveldata.xml`. Checks: `--paint-material <name part>` paints that material with `--paint x,y` and
  turns `--add-surface` on.
  * **More materials** (second tick box): the materials of the other worlds' cells and every
    `materials/aa_terrain/**.mtl` of the pak that no world uses (about 280 more than main_world's 317). Their elements
    come from the other world's `leveldata.xml`, or are written with the common 0.2 detail scale. These textures are not
    in this world's layer arrays, so the brush paints them into the heightmap at once; when the stroke ends the editor
    rebuilds the layer library (it reads the overlay's `leveldata.xml` files, which now name them) and reloads the
    terrain cells and their objects (`surface_add::reload_terrain`, a same-world version of `aa_world::switch`), which
    get the unsaved edits back. Checked: a pak-only road material painted into cell (15, 14) draws in the same session
    (325 materials after the reload instead of 324), with the edit still unsaved.
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
`EntityGuid`). Decals, roads, water volumes, interior brushes and new objects are in the window "More objects" (below).
Read-only: playing animated CGAs, voxels.

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

### More objects

`objects_extra.rs`. In Objects mode the button "More objects" at the bottom (or `--more-objects`) opens a window for
the cell under the cursor:

* **New model / New vegetation.** A searchable list of the pak's 10,055 `objects/**.cgf` models (LODs left out), or
  the world's vegetation groups. The picked model follows the cursor as a ghost (`[` `]` turn); a click places it
  (Shift keeps placing, Esc stops). A brush goes into `brush.dat` (the model is appended to `statobjs.dat`, written at
  once, and to `object.dat`'s model table, so both tables keep their order; an existing entry is reused) or into the
  `object.dat` octree in cells without `brush.dat`. The record is a copy of one of the cell's records with model,
  matrix, box (the model's LOD 0 bounds) and material -1 (the model's own `.mtl`). Vegetation copies a
  `vegetation.dat` record with the group set. The new object is then an ordinary world object (select, drag, turn,
  delete, Ctrl+S). There is no separate preview pane: the ghost in the world is the preview ("Model preview" shows
  database rows only).
* **Decals, Roads, Water** (`object.dat` item types 9, 13, 11; `objects_extra/shape.rs`). The list shows the cell's
  objects with coloured outlines; the selected one is white. Move (X / Y / Z buttons by a step), **Copy** (a new octree
  item 10 m east, then drag it into place) or delete. The selected item has handles in the view:
  * white: moves the whole object (box, position, road spline points, water outline points and plane). Decals and
    roads follow the ground; water keeps its surface height (use the Z buttons for that);
  * yellow (roads): a spline node, i.e. both edge points of it, so the road bends and keeps its width;
  * small blue: one road edge point (the width there) or one water outline point. The water's physics outline point
    moves with it when the two outlines have the same points.
  The box is refitted to the points after each change. **Undo** in the window undoes these changes one by one.
  The octree the Objects tools save is edited, so untouched bytes stay the same.
  **Drawing.** Water is meshed again at once: the cell's edited `object.dat` goes through `aa-world`'s parser and
  water mesher, the editor draws the result with the streamer's water materials and hides the streamer's water of
  that cell until the next reload. Roads and decals are built by `aa-world` code that is private to it, so with
  **Redraw roads and decals at once** (on by default) a change to one (a handle let go, a step, copy, delete or
  undo) saves the objects to the overlay and reloads the loaded cells' objects once the save is written (a second
  or two). Untick it to keep them unsaved: the outline shows the edit and **Save and redraw** draws it.
* **Interior brushes** (`visareas.dat`). Each area / portal record holds its own small octree. Its brushes are matched
  to the drawn brushes by model and position; they move and delete like the others, and the drawn brush follows at
  once. `visareas.dat` is written next to the cell's other files on Ctrl+S (record lengths and the header's compiled
  size recomputed, everything else verbatim). Parse -> write is byte-identical (tests `visareas_round_trip`, every
  25th main_world file, and `other_worlds_round_trip`).
* Checks: `--place-model <game/objects/...cgf | veg:<group>> --place-at x,y[,z]` places one object 8 s after start;
  `--extra-move <decal|road|water|interior>[#n]:dx,dy,dz` moves the n-th object of that kind in the camera's cell (no
  numbers: deletes it; with `--extra-grip <i>` only its point `i`); add `--objects-save` to save.
* Not editable: decal size and projection, voxels, and the area / portal shapes of `visareas.dat` (only its brushes).
* The check `--extra-move` also logs where the item is now, and redraws roads and decals like the window does.
  Octree node boxes are not refitted (a copy goes to the root node, whose box covers the cell).

## Worlds checked

The tools assume only the cell / block layout, so any world in `game/worlds/` loads (the world menu, or `--world
<name>` with `--spawn-dir <copy of Data/Worlds/name>`). Every world of the pak is checked by the test
`every_world_round_trip`: each cell's `object.dat`, `visareas.dat`, `brush.dat` and `vegetation.dat` write back byte for
byte, and an unedited `heightmap.dat` written by the editor reads back within one quantisation step (main_world too with
`AA_EDITOR_ALL_WORLDS=1`). Each world was also opened in the editor (see the table below). Earlier hands-on checks:

| World | What was run |
|---|---|
| `main_world` | everything (all features were built on it) |
| `instance_library` (interior dungeon) | loads and renders (25-30 fps, dev build); an interior bookshelf deleted, saved and still gone after a restart; a new model placed in a `brush.dat` cell, saved and reloaded |
| `instance_prologue` (island, 4 cells) | loads (70-80 fps); sculpt, save and "Export terrain to server" with the navmesh (2 files patched, only Z bytes changed); navmesh offsets checked by a test |
| `instance_sal_temple`, `arche_mall_world` (and the two above) | every `object.dat` and readable `visareas.dat` round-trips byte-identical (test `other_worlds_round_trip`) |
| all 17 worlds | opened with a screenshot each: no panics, no errors, 25-120 fps (dev build) |
| `arche_mall_world`, `instance_hadir_farm` | open as a white / beige wall: their zone `time_of_day.xml` pins the clock (13:00 / 8:30) with a volumetric fog of global density 5 (main_world zones: 0.4-1.5) and white or pale fog colours (`arche_mall_world` is the cash shop's showroom backdrop). That is the data, drawn as `aa-world` draws it; turn **View → Fog** off (or start with `--no-fog`) to edit them. arche_mall's town square runs at about 16 fps once its 196 NPCs are loaded (25 without them; the animation rate makes no difference, it is the characters' draw cost) and 4-9 fps for the 25 s they take to load |

Only Windows was used.

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

## Model preview

Menu bar "Model preview" (or `--preview <npcs|doodads|items|quests>:<id>`, which opens it on that row) shows a
turntable render of one row. Code: `crates/aa-editor/src/model_preview.rs`.

* **What it shows.** With "follow" ticked: the browser row the pointer rests on (0.3 s), else a row just opened in the
  row editor ("Preview" there forces it), else a spawn just selected in the world. Untick "follow" to keep the current
  model.
* **NPCs:** the same model as the world (the dressed humanoid with its carried weapons, a creature with its idle
  animation, a static model or prefab), scaled by `npcs.scale`.
* **Doodads:** the `.cgf` or the prefab's parts.
* **World objects:** in Objects mode, the model of the selected brush, vegetation instance or entity prop
  (`--preview-open` opens the window at start).
* **Items:** the icon (`items.icon_id` -> `icons.filename` under `ui/icon/`) plus:
  * weapons: `item_weapons` -> `item_assets.path` (a `.cgf`, or a `.cdf` carrier with its parts), stood up on its
    longest axis;
  * armour and clothing: worn on the default body of the race / gender picked in the window and nothing else. When
    that body has no meshes for the item, the first race / gender that has them is used, and the note says so;
  * other items: the icon only.
* **Quests:** the NPC who gives the quest (`quest_acts` `QuestActConAcceptNpc`), else the one it is turned in to
  (`QuestActConReportNpc`), else the first `quest_components.npc_id`.
* **View.** Drag to orbit, wheel to zoom, "turn" spins it, "Reset view" refits. The camera frames the model's mesh
  bounds. Skinned bodies have none, so their bones stand in for them.
* **How it renders.** The models spawn at the origin on render layer 7. Only the preview camera (a 640 x 640 image
  shown in the window, with its own ambient light) and its two directional lights see that layer, so nothing shows in
  the world view and the world's sun, fog and post effects stay off it. The camera is active only while the window is
  open, and a model is loaded only then.

## Worlds and navigation

* **World menu** (🌐 in the menu bar): the worlds of the pak (`worlds/<name>/world.g`). Picking one restarts the
  editor on it, because every tool keeps per-world state. With unsaved changes it asks first: save all and switch,
  discard and switch, or cancel. The new editor keeps `--db`, `--read-only-db`, `--overlay`, `--mode` and the Play on
  server flags. `--spawn-dir` and `--mirror` become `<parent>/<new world>`, so a server `Data/Worlds/<world>` layout
  maps directly. If that spawn folder doesn't exist yet (a scratch copy), it starts as a copy of the lane's files.
  The camera starts at the server's start point (`Worlds/world_spawns.json`), else in the middle of the world.
* **Gizmo** (top-right corner of the 3D view; `nav.rs`):
  * drag the ball to orbit around the point in the middle of the view (where the view's centre meets the ground);
  * click X / Y / Z to look along that axis (X east, Y north; Z is the top view);
  * drag 🔍 up / down to zoom and ✋ to pan;
  * 🗺 opens the teleport window.
* **Teleport** (M or 🗺; `teleport.rs`):
  * **Map:** a shaded relief of the terrain heights, one pixel per 32 m, built in the background the first time it
    opens (about 2 s for main_world). Drag to pan, use the wheel to zoom, click to teleport. Clicking a dot teleports
    to that place. The red triangle is the camera.
  * **Places:**
    * zones, at the sector nearest the middle of each zone group's sectors (`world.xml`), named after the zone group
      in the game database;
    * the server's respawn, recall and world gate points (`Data/Portal/*.json`) that lie on this world's ground;
    * the start point;
    * bookmarks (★ Bookmark camera; kept per world in `data/local/editor/bookmarks.json`);
    * with a search of two or more letters, NPC and doodad spawns by name or id (clicking one selects and focuses it).
  * **Go to:** `x, y` (the ground there) or `x, y, z`, as the status bar shows them.

## AI paths

Paths mode (5, `paths.rs`) shows and edits the routes NPCs walk: the AAEmu server's `Data/Path/<name>.path` files, one
point per line `action|x|y|z|param` (cry coordinates; `action` empty or `DisableLoop`, `EnableLoop`, `Speed`,
`ActorFlags`, with `param` its value), as `aaemu-game/src/systems/ai/path_handler.rs` reads them. An NPC follows a
route when its spawn entry says `"FollowPath": "<name>"`, or when a FollowPath AI command (`ai_commands.cmd_id = 2`,
`param2` = the name) of a command set runs it (quests, skills). A name used in several zones can ship as
`<name>@<zone>.path` variants: the server, and the editor's drawing, take the one passing closest to the NPC.

* **In the view.** The routes within 600 m of the camera (the distance is in the panel): green start point, red end,
  arrows along the way, a magenta stem on points with an action, magenta lines from the NPCs that follow the route to
  its start. The selected route is yellow with numbered points. In every mode, the route of the selected NPC is drawn.
* **Editing.** Click a point to select it and drag it (over the ground, keeping its height above it; untick "drag on
  the ground" to keep Z). Shift+click the ground adds a point after the selected one, or at the end. Delete removes the
  selected point, F focuses it, Esc drops the point selection, Ctrl+Z / Ctrl+Y undo / redo route edits.
* **Panel** ("AI paths"):
  * the routes, nearest first (search by name, or untick "near the camera" for all), with their point count, how many
    spawns and AI commands use them and the distance;
  * **New route** (empty: Shift+click to add points), **Duplicate**, **Delete route** (the file goes on save);
  * the selected route: rename (spawns that name it follow the new name; with "renaming also updates the N AI
    command(s)" ticked, the default, so do the database's commands), Focus, ▶ Walk (a marker walks it at 3 m/s),
    Reverse, Close loop (the first point again at the end), Snap to ground;
  * **Used by**: the spawns (click to select one) and "Selected NPC follows <route>"; and the `ai_commands` follow-route
    rows (`cmd_id` 2, `param2` = the route) naming it, each with its set id, set name and row id: re-point a row to
    another route, remove it (✖), or add a follow command to any command set by its id ("Add follow command", only
    for ids in `ai_command_sets`; new rows get `param1` 1, the common value, and the next id). These count as unsaved
    and are written to the game database with the routes (one transaction, only `cmd_id` 2 rows); with
    `--read-only-db` they are reported and not written;
  * the points: action, param, X / Y / Z, insert after, move up / down, remove.
* **NPC inspector** (Select mode): an NPC's "AI path" with a link to its route, ✖ to stop it following one, "Follow
  <selected route>", and "New route from here" (a new route `editor_<npc>_NN` starting at the NPC, which then follows
  it).
* **Saving** (Ctrl+S, File → Save AI paths, or the panel): every changed route is written to the path folder, CRLF, the
  untouched coordinates as they were written; a renamed or deleted route's old file is removed. The first overwrite or
  removal of a file keeps `<name>.path.editor-orig`. Saved files are copied to the `Path` folder of each spawn mirror
  (`<mirror>/../../Path`). `FollowPath` changes are saved with the spawns. **Restart the server** to load them.
* **Where routes are read from.** `--path-dir <dir>`; else `Data/Path` beside the spawn folder's `Data/Worlds` (the
  server layout: the lane's source tree by default); else a `Path` folder beside the spawn folder (a scratch copy,
  filled from the lane's routes the first time).
* Checks: `--mode paths`, `--path <name>` (select and focus that route), `--paths`.
* Tests: route files round-trip (the lane's 334 routes write back byte for byte), create / save / rename / delete with
  backups, `@zone` variants resolve to the nearest, and spawns read, set and clear `FollowPath`.

## Housing zones

Housing mode (key `6`, `--mode housing`) edits where players may build and what. Module `housing.rs`.

* **The data.** A housing zone is an `AreaShape` entity of a world zone's `level_design/zone/<key>/client/housing_area.xml`
  (113 files in the pak, 384 zones, 380 of them in main_world): a name, `Pos`, `cellX` / `cellY` and the corners
  relative to `Pos`. The server (`aaemu-game/src/systems/npcs/subzone.rs`) puts a corner at `(zone origin + cell) ×
  1024 + Pos + point`, the origin being `originX` / `originY` of the zone's `<Zone>` in `world.xml`, takes the area id
  from `value1` and tests a building's position against the polygon in 2D. The area id is a `housing_areas` row, which
  names a **rule set** (`housing_groups`) and its **allowed building categories** (`housing_group_categories`, the
  `housings.category_id` of the designs). `placement.rs` lets a building stand when a rule set of an area under it
  allows the category and the player passes its conditions: `houseless` (only players who own no building),
  `existing_category_id` (not players who already own a building of that category). `can_extend` off makes the
  building expire instead of paying more tax (the beginner plots). `doodad_id`, `allowed_tax_delay_week` and
  `max_construct_count` are client data the server does not use; they are editable all the same.
* **In the view**: the zones within a radius of the camera (800 m by default) are drawn along the ground, coloured by
  rule set, with posts at the corners and a label (name, area id, rule set). Click a zone to select it (yellow, with
  numbered corners); drag a corner to move it, drag inside to move the whole zone; Shift+click an edge (or anywhere,
  for the selected zone's nearest edge) adds a corner; Delete removes the selected corner (a zone keeps 3). F frames
  the zone (or focuses the corner). Ctrl+Z / Ctrl+Y undo and redo zone and rule edits together.
* **Draw new zone**: click the corners on the ground, Enter (or Finish) closes it, Esc cancels, Backspace removes the
  last corner. It goes into the file of the world zone owning the 64 m sector under the first corner (the file is
  created if that zone had none), named `LevelDesignShape_<zone>_editor_<n>`, with the next `EntityId` and a new
  `EntityGuid`, and gets a new `housing_areas` row (the next free id over the database and all worlds' files, the
  zone group's name, the shape name as comment) with the rule set picked beside the button.
* **Window "Housing zones", Zones tab**: search (name or area id) and the list near the camera; the selected zone's
  name (renaming also renames its area row's comment), area id (point it at another row; zones sharing a row are
  counted), rule set of its area row (a shared row changes for every zone using it), what that rule set allows,
  Focus, Duplicate (a copy 20 m east with its own area row), Delete zone (its area row goes too when no zone in any
  world uses it any more), and the corners (X / Y, remove). A zone whose area id has no row gets "Create the row".
* **Rule sets tab**: the 15 rule sets with how many areas use them; the selected one's English name and description
  (`localized_texts`), the conditions above, and the allowed categories (named after their designs, e.g. "11
  Swept-Roof Manse, Villa (56 designs)") with add / remove. "New rule set" copies the selected one with the next id;
  "Delete rule set" only for one no area uses.
* **Saving** (Ctrl+S, File → Save housing zones, or the window): each changed zone file is written to the pak overlay
  (`<overlay>/worlds/<world>/level_design/zone/<key>/client/housing_area.xml`, what the client reads) and to the
  server's `<server-data>/ClientOverlay/game/worlds/...` (first overwrite keeps `housing_area.xml.editor-orig`), and
  that folder is listed first in `ClientData.Sources` of the server config, as for "Server terrain" (`--server-data`,
  `--server-config`). The rule changes go to the database in one transaction (booleans as `'t'` / `'f'` like the
  rest, new rows with the next id); `--read-only-db` refuses them and says so. **Restart the server** to load both.
* **Files written as they were.** An untouched zone is written back as its original lines; a changed one in the same
  layout, every number that did not change in its original text (new ones as `%.8g`, like the original editor), the
  file's own line ending (6 files use CRLF) and the `Rotate` of the 4 zones that have one (a 1° tilt the server
  ignores). Test: all 113 files of the pak, regenerated from their parsed zones, are byte-identical. A moved zone's
  world position is kept in `f32` (as the server tests it), so its `Pos` can shift by a tenth of a millimetre.
* Checks: `--housing <area id>` starts in Housing mode with that zone selected and framed; `--housing-nudge dx,dy`
  then moves it and saves (with scratch `--overlay`, `--server-data` and `--server-config`); `--housing-tab rules`.
  Checked: area 212 ("terrorhousing1", Luxury Housing Province) follows its island's grass edge; nudged 30 m east and
  saved, the files in the overlay and the scratch `ClientOverlay` differ from the pak's only in that zone's `Pos`, and
  the scratch config lists the overlay. 42-88 fps (dev build).

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

* Roads and decals redraw by saving the objects (on by default) or with "Save and redraw"; there is no unsaved live
  preview of their drawn shape. Decal size / projection and the area / portal shapes of `visareas.dat` are not
  editable. The octree node boxes of `object.dat` are not refitted.
* A surface type from another world or the pak reloads the terrain when the stroke ends (a few seconds). Brand-new
  textures (files not in the pak) cannot be added. Surface painting changes the detail layer only; the cover colour
  needs the Tint brush.
* The navmesh follows height edits (heights, blocked and reopened links, split triangles) but `areasmission` (forbidden
  areas) is not touched, and nothing was run against the server's A* yet: the files were checked with the server's
  reader rules (links within a file, nodes merged by XY) by tests and a script, not by NPCs walking.
* AI paths: the server reads routes and AI commands at start.
* Housing zones: the server reads the zone files and rules at start. The editor lists the zones of the world it runs
  on; area rows used by other worlds' zones are never deleted with a zone here. What the client does with a rule
  set's `doodad_id`, tax delay and construct counts was not checked in the client.
* Windows only.
* Play (F5) runs inside the editor, without the server: no NPC AI, no quests. "Play on server" (below) runs the real client against the lane server for that.
* Model preview: armour is shown alone on a bare body, without the rest of a set. Rigid bone attachments of creatures
  are missing, as they are in the world.
