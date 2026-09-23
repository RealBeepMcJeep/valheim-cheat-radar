# Rust Valheim backup cheat scanner

Standalone, read-only scanner for Valheim `.tar.zst` backups and `.fch` character profiles. It invokes `zstd -dc`, streams tar records, parses world v37/v41 records, and never extracts or edits saves.

## Run

```text
cargo run --release -- --archive-dir C:\path\to\backups --prefab-names prefab_names.txt --output-dir reports-rust --character C:\path\to\backups\character-saves\active.fch --character-history-dir C:\path\to\steam\characters --validate
```

Without `--character`, the scanner chooses the deterministically first `.fch` in `<archive-dir>/character-saves`. Without `--character-history-dir`, history is read from that same directory. History inputs are read-only; the canonical file was already copied by the parent before scanning.

`--validate` checks generic, world-independent invariants (unique snapshot labels, internally consistent item counts) plus basic structural expectations for the canonical profile. Expectations that describe one particular save set — exact ZDO/evidence counts and delta counts — are not properties of the format, so they live in an optional local fixture instead of the source: pass `--oracle FILE`, or leave an `oracle.local.txt` beside the archives and it is picked up automatically. The fixture grammar is documented on `validate_oracle_fixture` in `src/lib.rs`.

Current chunks must be v41, the legacy database must be v37, and legacy item version 106 plus compact item/inventory versions 107 and 109 are explicitly supported. Profile v46 and playerData v33 are supported. Future or unknown versions are rejected rather than interpreted as current. Invalid profile hashes are untrusted and their parsed values are suppressed.

Outputs are deterministic and include `CHEAT_AUDIT.md`, `cheat-audit.json`, `world-evidence.csv`, and `character-evidence.csv`. The world CSV is a consolidated table over the most recent scanned snapshots, keyed by exact evidence identity excluding stack; each row has per-snapshot presence and stack columns. Reports use logical source labels only, never local absolute paths, account IDs, or raw player IDs.

World metadata found beside the chunks is reported too: the Markdown report gains a *World metadata* table (world name, version, seed, player count, progression-flag count) plus the latest snapshot's progression flags, and the JSON carries `world_name`, `world_seed`, `world_player_count`, `global_keys`, and `world_metadata_error` per archive. Steam ids and character names from the world file are never emitted.

The scanner excludes `dathost_settings_backup.json`, reports custom-data key names only, and does not implement save mutation or binary editing. Cross-save object identity is approximate because chunk records omit persistent ZDOIDs. Character lineage uses embedded playerID internally and reports only a same-lineage label.
