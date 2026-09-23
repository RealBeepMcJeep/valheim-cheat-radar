# TODO

Living backlog. Completed work is recorded in `CHANGELOG.md`; plans live in `plans/`.

## Focus (owner, 2026-09-23)

- The **standalone single-page web app** is the priority. Save parsing stays in Rust (that is where it
  is efficient and already shared by native and WASM); the CLI keeps working and nothing existing gets
  removed, but CSV/Markdown report polish is no longer a priority.
- Biomes on the map: content-based inference (a+b below) is wanted ASAP; the accurate seed/minimap path
  is explicitly deferred.

## Data-table lifecycle and size (settled with the owner, 2026-09-23)

- **Gzip the tables for the app**: 726 KB of names → 136 KB, biomes 23 KB → 4 KB, produced by
  `web/scripts/build-wasm.mjs` into a gitignored directory so nothing is duplicated in git. The native
  CLI keeps reading the plain committed files.
- **Lazy-load on the first ingested world file**, not at app start, and **drop both tables once the
  report has been built** — after parsing they are no longer needed (names are already strings in the
  report, and the map carries per-cell biomes). Re-inflating for a later scan costs ~10 ms, so dropping
  them is free.
- **Prune only what is demonstrably useless** — instance-suffix names (`Thing.047`), `DEF-*`, and 1–2
  character part names. Gzip already makes the full table cheap, so measure before cutting more.
- **Heightmap**: a true heightmap is not stored in a save (terrain comes from the generator). Cheap
  option from data we already have: per-cell ZDO altitude (mean/max `y`) drawn as relief shading —
  roughly ten lines in the tally plus a layer. The accurate version is the deferred seed/minimap path.

## UI plan (settled with the owner, 2026-09-23)

- **World metadata lives in two places**: a compact expandable `World details` panel in the masthead
  (newest snapshot: world name, version, seed, player count, progression-flag count, and the flags
  themselves), and the same fields per archive on the existing Timeline cards. Render parse errors only
  when present, next to the archive they belong to.
- **Seed is shown plainly** with a copy control (it is the user's own data, and it is useful), and it
  stays unmasked in exports.
- **Progression flags stay metadata**: they do not become evidence rows and the evidence CSV keeps its
  current shape.
- **Biome layer reuses the existing 64 m grid**, drawn as biome fill *under* the density shading, with a
  Biome / Density / Both control and a legend.
- **Verdict rule** (implemented): at least three single-biome objects' worth of weight and a 60% share
  for one biome, otherwise the cell stays blank.
- **Generated tables are committed** (`prefab_names.txt`, `prefab_biomes.txt`) and the name table is
  expanded from the same extraction, with provenance documented in `README.md`.
- **Multi-archive discoverability**: add an inline hint when only one archive is loaded, explaining that
  adding an earlier save classifies rows as `new` / `persisted` / `removed_or_cleared`.

## Character backup convention

Active profile plus its lineage is copied from the Steam Cloud profile directory
(`…\Steam\userdata\<id>\892970\remote\characters`) into
`C:\Users\user\Downloads\valheim-backups2\character-saves\`, and one date-prefixed snapshot per save
(`<YYYY-MM-DD>__<original name>`) goes into `character-saves\history\`. Scanning the history as a
timeline is what exercises profile reading over time:

```text
cargo run --release -- --archive-dir C:\Users\user\Downloads\valheim-backups2 `
  --prefab-names prefab_names.txt `
  --character      C:\Users\user\Downloads\valheim-backups2\character-saves\mustarðsmaðr.fch `
  --character-history-dir C:\Users\user\Downloads\valheim-backups2\character-saves\history
```

## Now — public release preparation

- [x] **Scrubbed save-derived data.** Real reports, character saves, and archives stay on disk
      outside the repository: `reports-rust/`, `*.tar.zst`, `character-saves/`, and the optional
      `oracle.local.txt` fixture are all gitignored. The report generators and `--validate` no longer
      assert one world's numbers or read one hard-coded set of snapshot labels.
- [x] Publish the public GitHub repo and deploy the built site to
      `realbeepmcjeep.github.io/valheim-cheat-radar/` (never the user-site repo). Verified live: the
      deployed WASM parses a synthetic legacy-v37 save end-to-end, and the user site is byte-for-byte
      unchanged.

## Deferred by the owner

- [ ] **License.** The repository is public with no `LICENSE`, so all rights are reserved by default;
      `Cargo.toml` also omits `license`/`repository` until this is decided.
- [ ] **Redacted sample saves/fixtures** so the tool can be demonstrated (and `--validate` exercised)
      without real save data. `oracle.local.txt` currently carries this save set's expectations and is
      deliberately gitignored.

## Verify the audit

- [x] Re-scanned the fresh backups in `C:\Users\user\Downloads\valheim-backups2` (17 archives;
      newest is the 2026-09-23 save). **Result: not clean.** The newest backup carries 142 cheated
      ZDO flags plus 109 direct-item and 26 container records; 140 of those records are new since
      2026-09-22, and `defeated_goblinking` appeared in the world's progression flags. The flagged
      content is still present and growing, so any cleanup did not hold.
- [ ] Character-resident verification needs a current profile: the newest backups have no
      `character-saves/`, so only the old `.fch` in this checkout was checked.
- [x] Re-ran the `--validate` sweep over the 15 local archives (fixture expectations still pass).

Working data for these checks lives outside the repository at
`C:\Users\user\Downloads\valheim-backups2` — 17 archives, newest is the 2026-09-23 save. The older
`valheim-backups` checkout was deleted during the migration; `reports-rust/` regenerates from either.

## Parser coverage gaps

- [x] **`.db2` is parsed.** Its `[u32 version][u64 uid][u32 payload length][gzip][trailer]` payload is
      inflated and the progression flags (`defeated_*`, `killed*`, `activebosses`, `event_*`, `hildir*`,
      `bosshildir*`) are reported. Only whitelisted namespaces are kept, so unrelated payload strings
      cannot reach a report, and a parse failure is recorded per archive instead of aborting the scan.
- [x] **`.fwl2` is parsed** for the world version, name, seed and player *count*. The player list's
      Steam ids and character names are read only far enough to count entries and are never retained.
- [ ] `prefab_names.txt` holds only 56 names while a real world contains ~700 distinct prefab hashes,
      so most objects render as `<unknown>`. Needs a decompiled prefab table (or a community name
      list) as input; the prototype's generator is gone with the old checkout.

## Browser / WebAssembly

- [x] Split the Rust project into byte/reader entry points while keeping the native CLI frontend.
- [x] Compile the parser to WebAssembly with a TypeScript bridge; no save parsing reimplemented in JS.
- [x] Streaming decompression, WASM parsing, and report generation in one module Web Worker with typed
      request IDs, progress, cancellation, and one-file-at-a-time processing.
- [x] Static, local-only SPA with relative asset URLs; no backend, no analytics, no remote assets.
- [x] Drag-and-drop for world `.tar.zst`/`.tar`/`.db`/`.chunk` and character `.fch`/`.fch.old`/`.fch.bak`.
- [x] Multiple sorted backups with stack-excluded identity, duplicate occurrence indexes, first/last-seen
      snapshots, and `observed`/`new`/`persisted`/`removed_or_cleared` statuses.
- [x] Filterable, sortable results table with prefab/item names and hashes, stack/slot, owner, chunk
      provenance, X/Y/Z, and JSON/CSV/Markdown export.
- [x] Cluster-by-location tree: cluster -> category -> record, with per-cluster type breakdown and
      expandable leaf details.
- [x] Interactive map view (Leaflet on `CRS.Simple`) with a ZDO-density heatmap, status-coloured
      evidence markers, cluster popups, and pan/zoom/pinch.
- [ ] Label each cluster with its chunk name and offer a spatial sort, so nearby sites sit together in
      the list. Chunk grouping itself is unsuitable — one chunk spans ~14 distinct sites and splits
      ~10% of true 30 m neighbours.
- [ ] Broader browser compatibility coverage.
- [ ] Surface world metadata in the browser UI: the world name, seed, player count and progression
      flags are in the browser report JSON but nothing renders them yet (the seed also unlocks an
      opt-in terrain/minimap overlay, see below).
- [x] Attach the single-file build automatically: publishing a release (or dispatching the workflow
      with a `tag` input) builds that tag and uploads `valheim-cheat-radar.html` to the release.

## Map: biome extraction findings (2026-09-23)

Environment: `D:\code\valheim-mods\Decompiled\Valheim\` is the full decompiled `assembly_valheim`
(631 files). The Biome flags enum + names live in `Heightmap.cs:16` (`None/Meadows=1/Swamp=2/
Mountain=4/BlackForest=8/Plains=0x10/AshLands=0x20/DeepNorth=0x40/Ocean=0x100/Mistlands=0x200`) with
the string mapping at `Heightmap.cs:1366`. The prefab↔biome tables are serialized MonoBehaviour
fields, not code:

| holder | fields | where it was found |
|---|---|---|
| `ZoneSystem` | `m_vegetation` (`ZoneVegetation.m_prefab` + `m_biome`), `m_locations` (`ZoneLocation.m_prefabName` + `m_biome`), `m_clutter` | bundle `…/SoftRef/Bundles/17245031` (461 KB) |
| `SpawnSystemList` | `m_spawners` (`SpawnData.m_prefab` + `m_biome`) | creature spawn areas in `…/Bundles/c4210710` |
| `ClutterSystem` | `m_clutter` (`Clutter.m_prefab` + `m_biome`) | same zone bundle |

The game install is at `I:\SteamLibrary\steamapps\common\Valheim` and its `valheim_Data` is mostly
stubs — content lives in 799 hash-named bundles under `StreamingAssets/SoftRef/Bundles/` (4.2 GB).
UnityPy 1.25.3 is installed for the system Python and already used by
`D:\code\valheim-mods\Scratch\mob-forms-prefab-parse.py`, so no new tooling is needed; a bundle loads
in ~5 s and every sampled MonoBehaviour had a readable typetree.

Measured so far (candidate tables written to `%TEMP%\vcradar-e2e\prefab_biomes_*.json`, not committed):

- `17245031`: **105** prefabs with a biome (98 single-biome) from `m_locations`/`m_vegetation`/`m_clutter`
  — and the assignments check out: `Crypt2..4` and `Greydwarf_camp*` → blackforest, `Grave1` → swamp,
  `DrakeNest01`/`Dragonqueen` → mountain, `GoblinCamp*`/`GoblinKing` → plains, `Eikthyrnir` → meadows.
- `c4210710`: **82** prefabs (59 single-biome) from creature spawn areas — `Wolf` → mountain,
  `Blob` → swamp, `Serpent` → ocean, `Asksvin` → ashlands.
- `m_locations` names come through directly (`m_prefabName`); `m_vegetation`/`m_clutter` entries only
  carry PPtr references. That turned out not to matter: the generator also names each biome-bearing
  component's *owner* object, so flora and saplings come through without any cross-bundle PPtr work.
  Two bundles alone yielded 368 tagged prefabs (271 single-biome) and 20,732 names.
- Reminder: item prefabs (what the audit flags: `Iron`, `Entrails`, …) carry no biome at all. The layer
  classifies world content, and a flagged item inherits the biome of the cell it sits in.

Current state: the map draws ZDO *density* per 64 m cell in one amber layer, plus evidence markers.
There is no biome estimate, and nothing stores per-cell prefab composition.

- [x] Record, per 64 m cell, a weighted tally of classified prefab biome evidence during parsing (the
      same loop that already builds the density grid), so no second pass over the world is needed.
      Done in `record_zdo_spatial`: weight is `12 / biomes`, and the tally rides along in
      `ArchiveScan.biomes`.
- [x] Generate a committed `prefab_biomes.txt` (prefab name → biome) from the game data, with a
      documented generator (`tools/extract-prefab-data.py`). Validated on the zone and creature
      bundles: crypts/greydwarf camps → blackforest, graves → swamp, drake nests → mountain, goblin
      camps → plains, wolves → mountain, serpents → ocean, flax/barley saplings → plains. The full
      799-bundle sweep produces the committed tables next.
- [x] Expand `prefab_names.txt` from the same extraction: 56 names → **31,601**, so `<unknown>` mostly
      disappears everywhere, not just on the map (712 KB raw, 136 KB gzipped).
- [ ] **Trees/plants are still untagged** (meadows reads low because of it). Progress and the exact
      wall hit on 2026-09-23:
      - The biome of flora lives on the scene's `ZoneSystem` entries, which point at prefabs in other
        bundles.
      - `m_FileID` indexes the **serialized file's externals**, *not* the AssetBundle dependency list —
        the two tables list the same bundles in different orders (externals[3] = `CAB-8923bd83` =
        bundle `c4210710`, while dependencies[2] = `CAB-cbd1a622` = an unrelated 480-object bundle).
        Fixed, and `--index-cabs` + `--resolve-scene` now implement it.
      - Path ids may be signed in the scene's typetree and unsigned from a bundle, so they are compared
        masked to 64 bits. Fixed.
      - Still only 5 of 112 references resolve: a direct probe *does* find the target
        (`m_clutter[0]` → `instanced_meadows_grass`, path id -3552536447561850049, in `c4210710`), yet the
        same lookup inside `resolve_scene` misses it. So the remaining bug is in that function's
        ref-walk or its grouping — next step is to print `{bundle: len(ids)}` and a few
        `(file_id, externals[file_id-1], path_id, found?)` tuples from inside it.
      - **Game-data route exhausted (2026-09-23).** The scene's `file_id` → cab mapping resolves the
        107 vegetation references to a bundle (`9fe0899c`) that holds **only Sprites/Textures** (247
        sprites, 9 textures, 1 atlas, no externals), and no bundle named after that cab (`6940c115…`)
        exists in either the client's 799 or the server's 798 bundles. So this build's scene references
        prefab content the shipped bundles no longer carry. Note the earlier "found it" was a false
        positive: path ids are unique only *within* a file, and the id I matched existed in a different
        bundle by coincidence.
      - Options, in order of preference: (a) a small, explicitly-labelled **curated flora hint table**
        (`name pattern → biome`, ~15 rules: Fir/Pine → blackforest, Beech/Birch → meadows, Oak →
        meadows/plains, VineAsh → ashlands, …) used *only* where the game table is silent; (b) obtain a
        matching-game-version bundle set or a community prefab→biome dataset; (c) leave trees untagged
        (honest, keeps the meadows skew). Spatial smoothing from neighbouring cells is deliberately not
        on the list: inference on inference.
      - **Source-code check (2026-09-23)**: no code-level tree↔biome table exists — tree names appear
        only in `PlayerStatType.cs` (chop stats), and there is no `Forest`/`m_biomeTrees` data. Biome at a
        coordinate is *computed*: `Heightmap.GetBiome` → `WorldGenerator.GetBiome(x, z, …)` (pure function
        of seed + position), which is the deferred accurate path, not a lookup table.
      - **New lead worth chasing first**: `World.m_biomeData` (`AltBiomeWorldData`) — `World.cs:56`,
        loaded via `AltBiomeWorldData.Load(binaryReader, world)` (`AltBiomeWorldData.cs:518`) with
        `Biomes`/`BiomeTypeInfo`/`BiomeSector` and `GenerateSectors()`. If that binary stream is one of
        the world files the scanner already reads (the `.db2` payload is the obvious candidate), the
        **biome map may be in the save itself**, which would beat content inference outright. Next step:
        look for biome/sector data in the `.db2` payload (we currently keep only whitelisted global
        keys, so anything else would have been ignored).
      - **Q14 answer: (a) approved.** Build the curated flora hint table (~15 name-pattern rules) as the
        pragmatic route, explicitly labelled heuristic and used only where the game table is silent. If
        the `m_biomeData` lead pans out, delete it in favour of the real data.
      - The 5 that do resolve from the scene's tables are real: `HugeStone1` → plains, `Waystone` →
        meadows,plains, `ormbunke_green_medium` → meadows, `rock_a` → plains.
      - Spatial smoothing of verdicts (fill blank cells from neighbours) is *later*, if ever: inference
        on inference.

- [x] Classify a cell only with enough evidence (≥3 single-biome objects' worth of weight) and a 60%
      share; otherwise leave it uncoloured — undeveloped, ocean and unexplored cells stay blank rather
      than be guessed. Implemented as `biome_verdict`; a cell popup with the counts behind the verdict
      is still to come with the UI work.
- [ ] Tune the flora hints against real saves: after adding them, coverage rose 40% → 54% and meadows
      went 158 → 1,291 cells (the skew is fixed), but **mountain fell 412 → 181**. Cause: with trees now
      voting in every forested cell and a two-biome hint weighting 6, tree votes can outvote sparse
      single-biome mountain markers (silver, obsidian, wolf, drake). Options: split Fir/Pine hints per
      variant, or ignore hint-derived votes in cells that already hold a strong single-biome game tag.
- [ ] Render biome fill plus density shading with a view control (Biome / Density / Both) and a legend,
      with one colour per biome (meadow green, black forest darker green, mountain white/grey, plains
      tan, swamp murky green, ashlands dark red, mistlands purple-grey, ocean deep blue).

- [ ] Cluster or spiderfy markers in the map view: a dense base stacks dozens of rings on one spot,
      which is clickable but unreadable at a glance.
- [ ] Give the ZDO-density layer an intensity gradient plus a metric scale/grid, so a hotspot reads as
      a hotspot and its coordinates can be matched against the in-game map.
- [ ] Measure on the real archives before trusting the layer: share of ZDO hashes classified, share of
      populated cells that get a verdict, and a spot check that unambiguous cells agree (silver ore →
      mountain, flax/barley → plains, crypt → swamp).

## Map: terrain from the seed / client cache (deferred by the owner)

Terrain is not stored in a save; the in-game map is generated procedurally from the seed by the game's
world generator, and the client caches the result locally as gzip'd 2048² RGBA. Deferred in favour of
content inference above:

- [ ] Optional overlay of the user's own local `cacheMinimapBiome` (gzip'd RGBA, decodable with the
      browser's built-in `DecompressionStream`). Requires calibrating `m_pixelSize`, which is serialized
      in a Unity prefab and not present in the decompiled source.
- [ ] Or run the generator from the parsed seed (`world_seed` is now reported) for the true biome map.

## Safe save scrubbing

- [ ] Keep scanning read-only by default; never overwrite an uploaded save.
- [ ] Offer an explicit "scrub a copy" workflow for world and character saves, with a preview/diff and a
      downloadable audit log.
- [ ] Let users choose between clearing cheat bits and removing contaminated item/object records;
      explain propagation and gameplay consequences.
- [ ] Recompute required wrappers/checksums (including `.fch` SHA-512), preserve unknown fields, and
      reparse/verify every generated save before download.
- [ ] Require an original backup and make destructive operations opt-in per row or selected group.
- [ ] Add round-trip fixtures and cross-check edited saves against the matching Valheim build.

## Forensic timeline research

- [x] Documented that chunked ZDO records omit persistent ZDOIDs and carry no universal creation time.
- [ ] Inventory prefab-specific temporal fields (spawn/start/queued times) and label them as hints.
- [ ] Extract save number, chunk revision, file timestamps, player history, and known temporal fields.
- [ ] Build first/last-seen timelines from multiple saves using approximate identity (prefab, position,
      slot/key, item metadata).
- [ ] For a single save, report only defensible temporal bounds and never invent creation dates.

## Presentation

- [x] Original CSS/art only; no redistributed game assets, logos, or unlicensed fonts.
