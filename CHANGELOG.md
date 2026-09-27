# Changelog

## Unreleased

### Fixed

- Mountain (and other real-marker) biome undercount: `prefab_biomes.txt` now marks flora-hint entries
  (trees the game's own tables never resolve) with a trailing `hint` column, and a cell's biome verdict
  is decided from real, game-tagged evidence alone whenever that alone clears the evidence bar — dense
  tree-hint votes only fill in a cell when real evidence is too thin to decide. On the owner's real
  saves this took mountain from 188 to 436 coloured cells (was 412 before the flora-hint feature, so
  this recovers and then some) while overall coverage still rose, 54% to 58% of populated cells.
- The biome legend's labels and counts no longer pile on top of each other: each entry was styled as
  a fixed-size status dot.

### Added

- **Experimental scrub in the browser:** after a scan, "Clear cheat flags in a copy" (behind an
  acknowledgement) hands back a `.tar.zst`, `.tar` or v41 `.chunk` in the same format and under the
  same file name as the input. For a backup, the decompressed tar is patched in place, so every entry
  (admin lists, host settings, world files), header and timestamp is identical and only the flag
  bytes differ; it is recompressed as one zstd frame with an XXH64 checksum (pure-Rust `ruzstd`,
  about zstd level 1, so larger than the host's file), then decoded again with fzstd and compared
  before it is offered. Checked on the owner's newest save with the real `zstd` and `tar`: the zstd
  frame passes `zstd -t`, 637 bytes change (all 1 -> 0), the 85-entry tar listing is identical, and the
  world files match the CLI scrub's byte for byte. Legacy `.db` worlds and `.fch` profiles are refused.
- **Mode A scrub (CLI):** `--scrub-world ARCHIVE --scrub-out DIR` clears every cheat flag the scanner
  finds — `cheated`/`cheatedQueued[+slot]` ZDO ints and each item's cheated bit — on a copy of a v41
  world, keeping every object. Each patch is a same-length in-place byte change, checked against the
  bytes actually there before it is applied; the output is then verified (only audited bytes differ,
  a re-parse finds zero flags with an unchanged ZDO count and layout, and every written file is read
  back) or deleted. It refuses to write into a non-empty directory or next to the input, fails closed
  on legacy or unrecognized world files, and writes `SCRUB_AUDIT.md` / `scrub-audit.json`. On the
  owner's newest save: 469 ZDO flags and 168 item bits cleared, exactly 637 bytes changed, and a local
  dedicated server loaded the result with the same log as the original.
- World details in the browser: world name, version, seed (with a copy control), player count and
  progression flags in a masthead panel for the newest save, and per archive on the Timeline cards.
- A hint, when only one save is loaded, that adding an earlier one classifies rows as new / persisted /
  removed_or_cleared.
- Location-tree clusters are labelled with their dominant chunk, and a "Nearby together" order chains
  neighbouring clusters.
- Map: clicking any cell shows its ZDO count and biome verdict or why there is none, whether the verdict
  needed flora-hint votes, and the top vote weights (`biome_detail` in the map JSON).
- Map: nearby evidence markers collapse into a count badge that splits as you zoom; clicking a badge
  lists every record in it.
- Map: density is a log-scaled multi-stop gradient with a legend, plus a 1000 m world grid and a scale.

- Biome inference from content for the map: every ZDO whose prefab the game tags with a biome votes
  for that biome in its 64 m cell, weighted `12 / biomes` so a prefab tagged with many biomes counts
  for less. A cell is coloured only with at least three single-biome objects' worth of weight and a 60%
  share for one biome; thinner cells stay blank. The map JSON carries `biomes` plus `biome_names`, and
  the CLI takes `--prefab-biomes FILE` (optional: without the table the layer is simply absent).
- `tools/extract-prefab-data.py`, which generates the committed `prefab_names.txt` and
  `prefab_biomes.txt` from the game's own bundles — game data, never save data. Provenance and the
  regeneration command are in `README.md`; the bits and thresholds are in `FORMAT.md`.
- `.fwl2` world metadata: world version, name, seed, and player *count*. The player list's Steam ids
  and character names are read only far enough to count entries and are never retained or reported.
- `.db2` progression flags (`defeated_*`, `killed*`, `activebosses`, `event_*`, `hildir*`,
  `bosshildir*`), inflated from the file's gzip payload. Only whitelisted key namespaces are kept, so
  unrelated payload strings cannot reach a report, and unknown shapes become a per-archive metadata
  error rather than a failed scan or a guess.
- World metadata in the reports: a *World metadata* table plus the latest snapshot's progression
  flags in Markdown, and `world_name` / `world_version` / `world_seed` / `world_player_count` /
  `global_keys` / `world_metadata_error` per archive in both JSON reporters.

### Notes

- Second runtime dependency: `ruzstd`, pure Rust, for the browser scrub's zstd output (+~110 KB of
  WASM).
- First runtime dependency: `flate2` with its pure-Rust backend, so gzip is handled by a standard
  implementation shared by the native CLI and the WASM build instead of a hand-rolled inflate or a
  `gzip` child process.

## 0.1.0 — initial public release

### Added

- Read-only Rust scanner/library for Valheim world archives and `.fch` character profiles:
  streaming tar with checksum and end-marker validation, legacy world v37 and chunked world v41
  parsing, cheated item and ZDO flags, queued crafting-station flags, container-inventory evidence,
  direct item evidence, and character-profile integrity verification.
- Deterministic Markdown, CSV, and JSON reports, including a consolidated evidence table that
  compares the most recent snapshots using stack-excluded identity.
- WebAssembly build of the same parser behind a `wasm-bindgen` `BrowserScanner`, plus a
  Preact + TypeScript + Vite SPA: drag-and-drop intake, one module Web Worker owning decompression,
  parsing and report generation, a filterable and sortable evidence table, a cluster-by-location
  tree, an interactive Leaflet map with a ZDO-density heatmap, and JSON/CSV/Markdown export.
- Generic structural `--validate` invariants for any save set, plus an optional local oracle
  fixture (`oracle.local.txt`, gitignored) for expectations that describe one private save set.

### Hardening

- Fail-closed handling for unsupported world, inventory, item, player-data, and profile versions.
- Malformed tar records, invalid checksums, nonzero trailing data, unsafe tar paths, and archives
  without a recognized parseable world payload are rejected rather than guessed.
- Invalid character hashes make a profile untrusted and suppress its parsed values.
- Bounded streaming decompression in the browser scanner, with input and decompressed ceilings.
- Markdown and CSV exports escape untrusted text; reports emit logical source labels instead of
  local absolute paths and never raw player IDs.
- Generated HTML renders only through a reviewed inert-DOM sanitizer; no direct `innerHTML` sinks.
- Relative production asset URLs with `base: './'` for static and subpath hosting; no backend,
  analytics, remote fonts, or runtime network requests.
- Regression tests across the Rust parser and the TypeScript UI.
