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

- [ ] Re-scan fresh backups after cleanup and confirm no world-resident or character-resident cheat
      flags remain.
- [ ] Re-run the full `cargo run --release -- --validate` sweep over the local archives; the last
      full run predates the recent parser changes.

## Parser coverage gaps

- [ ] **`.db2` is not parsed at all.** It is a gzip-compressed blob holding the world's global keys
      (`killedtroll`, `defeated_*`, `activebosses`, …) — genuine progression/cheat signal that the
      scanner currently ignores entirely.
- [ ] **`.fwl2` is not parsed.** It carries the world name and seed (and a player list that must never
      be reported). The seed would let a user render a real external map.
- [ ] `prefab_names.txt` holds only 56 names while a real world contains ~700 distinct prefab hashes,
      so most objects render as `<unknown>`. Generate a full name table from the decompiled source.

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
- [x] Attach the single-file build automatically: publishing a release (or dispatching the workflow
      with a `tag` input) builds that tag and uploads `valheim-cheat-radar.html` to the release.

## Map: making it a *terrain* map

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
