# Valheim Cheat Radar web app

Static, local-only Preact + TypeScript + Vite UI for the Rust parser. A single module Web Worker owns `fzstd` streaming decompression, the `wasm-bindgen` scanner, and report generation; the UI only queues files and receives progress/results. Files are transferred to the worker one at a time and are never uploaded. This is an unofficial, read-only Valheim tool.

## Commands

From the repository root:

```text
cargo fmt --check
cargo test --release
cargo clippy --release -- -D warnings
cd web
npm install
npm test
npm run typecheck
npm run build
```

Build the actual WebAssembly package before a clean web build:

```text
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --version 0.12.1 --root .tools/wasm-pack --locked
cd web
npm run wasm:build
npm run build
```

Build the single-file download locally (releases get it automatically):

```text
cd web
npm run build:single   # writes dist-single/valheim-cheat-radar.html
```

Publishing a release triggers the `Build and publish` workflow, which builds that tag and uploads
`valheim-cheat-radar.html` to the release. To re-attach an asset without creating a release, run the
workflow manually with its `tag` input; run it with no input to just (re)deploy the site. Release
builds and site deploys use separate concurrency groups, so neither can cancel the other.

`dist-single/valheim-cheat-radar.html` is one self-contained file: CSS, bundle, module worker, and the WASM (base64) are all inlined, so it works when opened directly from disk. Two browser facts shape that build, both verified in a real browser: a `blob:` **module** worker is refused from a `file://` page while a `data:` module worker runs, and the worker therefore resolves the WASM from a `data:application/wasm` URL rather than a relative path. `web/scripts/build-single-file.mjs` fails loudly if Vite's output shape changes instead of emitting a broken file, and the deploy workflow runs it on every push so drift surfaces immediately.

`web/scripts/build-wasm.mjs` uses `.tools/wasm-pack/bin/wasm-pack.exe` when present, otherwise `wasm-pack` from `PATH`. `.tools/`, `target/`, `web/node_modules/`, and `web/dist/` are ignored. The checked-in `web/wasm/pkg/` is the generated deployment input; regenerate it when Rust changes. `cargo check --target wasm32-unknown-unknown --release` also checks the library and its no-op WASM binary entry point.

## Supported inputs

- `.tar.zst`: JavaScript decompresses it in the worker, then Rust parses the decompressed tar.
- `.tar`: decompressed tar input.
- `.db`: legacy v37 `Dedicated.db`-compatible bytes.
- `.chunk`: current v41 chunk bytes with a defensible Valheim chunk filename such as `00_00__1_1.chunk`; standalone chunks have no metadata revision.
- `.fch`: profile wrapper with SHA-512 validation, profile v46, playerData v33, and inventory versions supported by the native parser.

Unknown/future versions, malformed tar records, invalid checksums, nonzero tar trailing data, invalid profile hashes, unsupported raw layouts, and browser tar files without a recognized parseable world payload are rejected rather than guessed. `.fch`, `.fch.old`, and `.fch.bak` profile names are accepted; backup suffixes are retained for comparison but never automatic canonical candidates. A single trusted, supported profile is selected automatically; multiple profiles require a unique newest `File.lastModified` or explicit UI selection. Unsupported/untrusted values are unavailable rather than clean zeros. No raw player IDs or absolute paths are emitted. Browser reports are generic: they carry no hard-coded snapshot labels or dates, no cleanup claims, and a CSV schema derived from the scanned files rather than a fixed set of snapshots.

## Reports and limits

The UI provides JSON, CSV, and Markdown downloads containing archive metadata, parsed evidence, statuses, stack-excluded identity, deterministic duplicate occurrence indexes, first/last-seen snapshots, character profiles, and approximate timeline comparisons. One save reports `observed`; multiple sorted saves classify each occurrence relative to the latest save as `new`, `persisted`, or `removed_or_cleared`. Failed files produce a prominent partial-report banner and retain exact per-file errors in the queue and exports.

`.tar.zst` input is fed to `fzstd.Decompress` in bounded compressed chunks and retained output is rejected before the configured 256 MiB decompressed ceiling is exceeded. The 128 MiB input and 256 MiB decompressed limits are conservative browser safety bounds that still cover the known roughly 17 MiB compressed / 27 MiB decompressed save. Peak memory is still higher than either limit: compressed input, streaming output chunks, one contiguous tar buffer required by the Rust parser, and WASM's input copy can overlap briefly. The worker reports a parsing stage after decompression and can be terminated by Cancel or Reset, but Rust tar scanning itself is synchronous inside that worker and cannot report finer-grained intra-file progress.

This MVP never edits or scrubs a save. It has no backend, analytics, remote fonts, CDN, or runtime network request. The table is scrollable rather than virtualized.

## Audit note

`npm audit --omit=dev --audit-level=high` currently reports 0 production vulnerabilities. Full `npm audit` reports two moderate dev-only Vitest advisories through `@vitest/mocker` (GHSA-82fw-gwwq-j7x9); npm offers a breaking `vitest@5.0.1` upgrade, so it was not applied to this MVP. The native CLI keeps its explicit `--validate` path, which checks generic structural invariants and then, when present, the gitignored local oracle fixture.
