# Valheim Cheat Radar

A read-only Valheim save auditor. It parses world backups (`.tar.zst`, `.tar`, legacy
`Dedicated.db`, chunked `.chunk`) and character profiles (`.fch`) and reports cheat evidence —
cheated item and ZDO flags, queued crafting-station flags, container and direct item evidence,
and character cheat indicators — without ever writing to a save.

Two front ends over one Rust parser:

- **`web/`** — a static, local-only browser app (WebAssembly + Preact). Drop files in; decompression,
  parsing, and report generation happen in a worker inside the tab. No backend, no uploads, no
  analytics. Live at <https://realbeepmcjeep.github.io/valheim-cheat-radar/>.
- **`src/`** — the native CLI and library, for scripting over a directory of archives.

There is also a **single-file download**: [Releases](https://github.com/RealBeepMcJeep/valheim-cheat-radar/releases)
has one self-contained `.html` (app, worker, and WASM inlined) that runs by double-clicking, with
nothing installed and no network access.

## Quick start

```text
# Browser app
cd web
npm ci
npm run wasm:build   # regenerates web/wasm/pkg from src/ (needs wasm-pack)
npm run dev          # http://localhost:5173

# Native CLI
cargo run --release -- --archive-dir C:\path\to\backups --output-dir reports-rust --validate
```

See `RUST_README.md` for CLI details, `WEB_README.md` for the app, and `FORMAT.md` for parsed
format notes. `TODO.md` is the living backlog; `CHANGELOG.md` is the history.

## Privacy

Input saves are read-only and never leave the machine. Reports use logical source labels instead of
absolute paths, and character lineage is compared by the embedded player ID but reported only as a
same-lineage label. Real save data, reports generated from it, and world-specific validation
expectations are all gitignored: see `.gitignore`, and the optional `oracle.local.txt` fixture
described in `RUST_README.md`.

## Unofficial

Unofficial fan-made tool. Valheim is a trademark of its respective owner. No game assets, logos,
fonts, or official screenshots are distributed here.
