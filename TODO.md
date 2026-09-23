# TODO

Living backlog. Completed work is recorded in `CHANGELOG.md`; plans live in `plans/`.

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

## Map: making it a *terrain* map

- [ ] Cluster or spiderfy markers in the map view: a dense base stacks dozens of rings on one spot,
      which is clickable but unreadable at a glance.
- [ ] Give the ZDO-density layer an intensity gradient plus a metric scale/grid, so a hotspot reads as
      a hotspot and its coordinates can be matched against the in-game map.

Terrain is not stored in a save; the in-game map is generated procedurally from the seed by the game's
world generator, and the client caches the result locally as gzip'd 2048² RGBA. Options if wanted later:

- [ ] Optional overlay of the user's own local `cacheMinimapBiome` (gzip'd RGBA, decodable with the
      browser's built-in `DecompressionStream`). Requires calibrating `m_pixelSize`, which is serialized
      in a Unity prefab and not present in the decompiled source.
- [ ] Estimate biome bounds from prefab distributions once the full prefab name table exists.

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
