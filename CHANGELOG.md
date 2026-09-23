# Changelog

## Unreleased

### Added

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
