# Parsed format notes

## World archives

- Archives are streamed through `zstd -dc` and a small tar reader with header checksums and required two-block end markers.
- Chunked world records require version 41; `Dedicated.db` is parsed only as legacy v37.
- Evidence keys are stable hashes: `cheated`, `cheatedQueued`, `itemData`, and `items`.
- Legacy item version 106 and compact item/inventory versions 107 and 109 are explicitly supported; unknown/future versions fail closed.
- Coordinates, chunk filename/version/size/revision, ZDO ordinal, owner hash/name, key hash/name, item fields, grid, quality, stack, variant, crafter name, world level, and custom-data key names are retained internally.
- `world-evidence.csv` consolidates logical evidence identity across the most recent scanned snapshots. Identity matching excludes stack and uses kind, owner hash, exact float-bit position, key, item, grid, quality, variant, crafter, and worldLevel. Stack is observation-only in per-snapshot columns.

## Character profiles

The `.fch` wrapper is `[payload length][ZPackage payload][SHA-512 length][SHA-512]`. Profile v46 contains ten 205-slot stat profiles and an optional playerData payload. The current playerData v33 and inventory v109 layouts are parsed read-only. `m_usedCheats`, the `Cheats` stat slot, selected known-command names, cheated inventory bits, and exact `bypasscheatchecks 1` are reported; arbitrary custom-data values are never reported.

A bad SHA-512 makes a profile untrusted and unsupported, with cheat values suppressed. Unsupported profile/playerData versions remain visible as untrusted rows instead of being interpreted as current. Steam Cloud history is compared with the canonical active profile by embedded `playerID`; reports expose only a same-lineage label and never the raw ID or local source path.
