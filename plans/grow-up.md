# Plan: grow the project up (migrate, publish, deploy)

Status: **decided — ready to execute** (two open items noted at the end)
Owner: pi-agent · Created: 2026-09-22

## Goal

Turn a working local prototype into a published open-source project:

1. Move to `D:\code\valheim-cheat-radar`.
2. Publish as a **public** GitHub repo on `RealBeepMcJeep`.
3. Every commit authored as `pi-agent`.
4. Build and deploy to `https://realbeepmcjeep.github.io/valheim-cheat-radar/`.

Hard constraint from the owner: **the repository must not contain the owner's save data.**

## Decisions (settled)

| # | Decision | Answer |
| --- | --- | --- |
| 1 | Git history | **Fresh.** One initial commit over scrubbed code. Save-derived data is in every existing commit, so history is discarded rather than rewritten. |
| 2 | Public contents | **(a) now** — code, SPA and docs only. A synthetic sample save (option c) may follow later. |
| 3 | Packaging | **(c)** `npm run dev` keeps working locally; CI deploys the app to Pages; a **single-file** build is produced separately (see open item B). |
| 4 | `--validate` | **(b)+(c)** — generic structural invariants stay in the repo; world-specific expectations move to a gitignored local fixture. |
| 5 | Commit identity | **`pi-agent <pi-agent@users.noreply.github.com>`**, applied via the pi harness shell prefix (see below). Global git config untouched. |
| 6 | Migration scope | Source + `.git`-less tree + local data move; `target/`, `node_modules/`, `.tools/`, `dist/` are rebuilt. Old directory kept until the new one passes. |

### Commit identity — implemented

`~/.pi/agent/settings.json` gained a `shellCommandPrefix` that exports
`GIT_AUTHOR_NAME/EMAIL` and `GIT_COMMITTER_NAME/EMAIL` for every shell command pi runs.
Verified that this sets both author *and* committer to `pi-agent` while
`git config --global user.name` stays `Nobody`. A backup of the original settings was
kept (`settings.json.bak-*`). **Requires a pi restart to take effect.**

## Verified facts (checked, not assumed)

| Fact | Value |
| --- | --- |
| `RealBeepMcJeep.github.io` | **Exists and is live** — the Pages user site, `main`, `index.html` + `tools/` |
| Existing project sites | `pokemon-checklist` -> `…github.io/pokemon-checklist/`, `mod-tracker` -> `…github.io/mod-tracker/` |
| `valheim-cheat-radar` repo | Does not exist |
| `gh` CLI | Authenticated as `RealBeepMcJeep`, scopes include **`workflow`** |
| Global git identity | `Nobody <nobody@nowhere.com>` — owner's deliberate default, must not be changed |
| `D:\code\valheim-cheat-radar` | Destination directory for the migrated repository |

The user site is occupied, so this project gets its **own** Pages site as a subpath. Matches the
pattern already used by `pokemon-checklist` and `mod-tracker`.

## Blocker being removed: save-derived data

Wider than "delete the reports" — it reaches into the core source, and exists in every commit.

| Location | What leaks |
| --- | --- |
| `reports-rust/CHEAT_AUDIT.md` | Player name, character backup filenames, save timestamps, base coordinates, a second player name |
| `reports-rust/character-evidence.csv` | Same, plus per-save inventory counts |
| `reports-rust/cheat-audit.json` | Same, plus world identifiers and an excluded-path name |
| `reports-rust/world-evidence.csv` | Per-object coordinates for the owner's base |
| `CHANGELOG.md`, `TODO.md` | Player name |
| `src/lib.rs` — `validate_oracle` | Hard-coded snapshot labels and that world's expected counts |
| `src/lib.rs` — Markdown/JSON generators | Hard-coded delta counts and that world's timeline and coordinate narrative, emitted into **every** report |
| `src/lib.rs` — `delta_statuses`, `consolidated_evidence` | Hard-coded snapshot labels for the compared snapshots |
| Not tracked, but local | `.tar.zst` archives, `character-saves/`, the five mockup HTMLs (they embed the audit rows) |

The two `src/lib.rs` items are also a **quality defect independent of privacy**: a general-purpose
tool must not assert one specific world's numbers.

## Phases

Corrected order — the scrub happens **before** the first commit, so nothing leaks into the new history.

### Phase 0 — Stage the destination

- [x] Create `D:\code\valheim-cheat-radar`.
- [x] Copy the working tree **without `.git`** (fresh history means the old history is discarded).
- [x] Copy local-only data the CLI needs: archives, `character-saves/`, `reports-rust/`.
- [x] Skip regenerables: `target/`, `node_modules/`, `web/dist/`, `.pi/`; `_bench/` (scratch
      benchmarks) also stayed behind. `.tools/wasm-pack` (5 MB) was copied so `npm run wasm:build`
      keeps working offline.

### Phase 1 — Scrub (before any commit)

- [x] Untrack `reports-rust/`; add it to `.gitignore`; keep the files locally. `character-saves/`,
      `*.tar.zst`, and the new `oracle.local.txt` are ignored for the same reason.
- [x] Strip player/character names: `CHANGELOG.md` was rewritten generically, `TODO.md` updated, and
      `FORMAT.md` lost its hard-coded snapshot list.
- [x] Remove the five mockup HTMLs from the working tree (they embed the audit); they remain in the
      old directory. Their generator in `tools/` went with them.
- [x] `src/lib.rs`: `--validate` keeps generic structural invariants; world-specific expectations
      moved to a local, gitignored fixture read via `--oracle FILE` (or `oracle.local.txt` beside the
      archives when present).
- [x] `src/lib.rs`: the hard-coded delta sentence, the world-specific narrative, and the hard-coded
      snapshot labels for delta status and the consolidated table are gone; those now derive from the
      scanned archives, and the CSV header set follows them.
- [x] Prove the scrub: zero hits in the tree *and* in history for player names, Steam IDs, world and
      archive IDs, evidence-count fingerprints, and all 570 distinct coordinate values taken from the
      real `world-evidence.csv`.

### Phase 2 — Fresh repository

- [x] `git init` in the new directory.
- [x] Confirm authorship is `pi-agent` **before** the first commit (`git var GIT_AUTHOR_IDENT` and
      `GIT_COMMITTER_IDENT` both reported `pi-agent <pi-agent@users.noreply.github.com>`).
- [x] Initial commit `2c22859` — 42 files.
- [x] Verify from the new location: `cargo fmt --check`, `cargo test --release` (24 passing),
      `cargo clippy --release -- -D warnings`, and in `web/`: `npm ci`, `npm test` (29 passing),
      `npm run build`. The shipped `web/wasm/pkg` was regenerated from the scrubbed sources and
      re-checked for stale strings.

### Phase 3 — Publish

- [x] `gh repo create RealBeepMcJeep/valheim-cheat-radar --public --source . --push`.
- [x] Confirm the remote file list contains no save-derived file (0 matches for `*.tar.zst`, `*.fch`,
      `reports-rust/`, `character-saves/`, `oracle.local.txt`).

### Phase 4 — CI and Pages

- [x] Workflow: Rust + `wasm32-unknown-unknown` + `wasm-pack`, `npm ci`, `npm test`,
      `npm run wasm:build`, `npm run build`, then `actions/deploy-pages`.
- [x] Deploy to the **project** site only: `https://realbeepmcjeep.github.io/valheim-cheat-radar/`
      (repo Pages set to `build_type: workflow`).
- [x] `.nojekyll` is written into `dist/` by the workflow so the asset directory is served verbatim.
- [x] Vite already uses `base: './'`, so subpath hosting needed no change.
- [x] Verify on the deployed site: the WASM asset is served as `application/wasm`; a synthetic
      legacy-v37 save injected through the page parsed end-to-end and reported
      `archives=synthetic-valid.tar:legacy_v37:zdo=0` with `tool=Valheim Cheat Radar`; and
      `https://realbeepmcjeep.github.io/` is byte-for-byte unchanged
      (sha256 `497f6871…f3f7a6`, repo HEAD still `6fa6aaa4`). Live verification also found and fixed
      a real defect: the worker masked wasm-bindgen's string errors (`74e7a8c`), so a rejected save
      now shows the parser's actual message.

### Phase 5 — Single-file build

- [x] One self-contained `.html` (`npm run build:single` → `web/dist-single/valheim-cheat-radar.html`):
      CSS, bundle, module worker, and the base64 WASM are all inlined. The worker became a
      `data:text/javascript` URL rather than a blob one: a real-browser check showed a `blob:`
      **module** worker is refused from a `file://` page while a `data:` module worker runs, and the
      worker's WASM lookup therefore resolves an absolute `data:application/wasm` URL. The script
      fails loudly if Vite's output shape drifts, and the deploy workflow runs it on every push.
      Verified by opening the built file from `file://` and scanning a save end-to-end.
- [x] Attached to GitHub Releases (`v0.1.0`), so a server admin can download one file and open it —
      automatically now: publishing a release builds that tag and uploads the file to it, and release
      runs are keyed to their own concurrency group so a docs push cannot cancel an asset upload.

## Open items

**A. Destination path — resolved.** The directory used is `D:\code\valheim-cheat-radar` (the plan
initially said `D:\code\valheim\valheim-cheat-radar`). Everything ran from there.

**B. Where the single-file build ships — resolved.** Pages keeps serving the normal bundle; the
single file ships as a **release artifact** (`v0.1.0` and later), which matches decision 3(c).

**C. Attribution nuance.** `pi-agent@users.noreply.github.com` is not registered to any GitHub
account, so commits will display as an unlinked author named `pi-agent` rather than being attributed
to `RealBeepMcJeep`. That matches "specific to the pi agent"; say the word if you want them linked to
your account instead.

**D. Licensing — owner's call, deliberately deferred.** The repository has no `LICENSE`, which means
all rights reserved by default. `Cargo.toml` therefore also has no `license`/`repository` fields.

**E. Local-only follow-ups.** `oracle.local.txt` (gitignored) carries the archive sweep expectations
for this save set, so `cargo run --release -- --validate` still checks real counts locally; a redacted
sample save would let the same happen for anyone else.
