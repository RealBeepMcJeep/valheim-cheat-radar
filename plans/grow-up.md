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
- [ ] Copy the working tree **without `.git`** (fresh history means the old history is discarded).
- [ ] Copy local-only data the CLI needs: archives, `character-saves/`, `reports-rust/`.
- [ ] Skip regenerables: `target/`, `node_modules/`, `.tools/`, `web/dist/`, `.pi/`.

### Phase 1 — Scrub (before any commit)

- [ ] Untrack `reports-rust/`; add to `.gitignore`; keep the files locally.
- [ ] Strip player/character names from `CHANGELOG.md`, `TODO.md`, and any fixture.
- [ ] Remove the five mockup HTMLs from the working tree (they embed the audit); they remain in the
      old directory.
- [ ] `src/lib.rs`: keep generic structural invariants in `--validate`; move world-specific
      expectations into a local, gitignored fixture the CLI reads when present.
- [ ] `src/lib.rs`: remove the hard-coded delta sentence from the Markdown report generator.
- [ ] Prove the scrub: grep the whole tree for player names, Steam IDs, the archive ID, and the base
      coordinates. Expect zero hits outside gitignored local data.

### Phase 2 — Fresh repository

- [ ] `git init` in the new directory.
- [ ] Confirm authorship is `pi-agent` **before** the first commit (env vars are live only after a pi
      restart — verify with `git var GIT_AUTHOR_IDENT`).
- [ ] Initial commit.
- [ ] Verify from the new location: `cargo test --release`, `cargo fmt --check`,
      `cargo clippy --release -- -D warnings`, `cd web && npm ci && npm test && npm run build`.

### Phase 3 — Publish

- [ ] `gh repo create RealBeepMcJeep/valheim-cheat-radar --public --source . --push`.
- [ ] Confirm the remote file list contains no save-derived file.

### Phase 4 — CI and Pages

- [ ] Workflow: Rust + `wasm32-unknown-unknown` + `wasm-pack`, `npm ci`, `npm run wasm:build`,
      `npm run build`, then `actions/deploy-pages`. Requires `workflow` scope (confirmed present).
- [ ] Deploy to the **project** site only: `https://realbeepmcjeep.github.io/valheim-cheat-radar/`.
- [ ] Add `.nojekyll` so the asset directory is served verbatim.
- [ ] Vite already uses `base: './'`, so subpath hosting needs no change.
- [ ] Verify: deployed site loads its WASM and parses a save; `https://realbeepmcjeep.github.io/`
      is unchanged.

### Phase 5 — Single-file build (deferred)

- [ ] Add a build that inlines everything into one `.html`: base64-inline the WASM (~215 KB -> ~287 KB)
      and convert the module worker to a blob URL.
- [ ] Attach it to GitHub Releases so a server admin can download one file and double-click it.

## Open items

**A. Preconditions for the handoff.** The next session is meant to run *from*
`D:\code\valheim\valheim-cheat-radar`, but that directory does not exist yet. Either the owner
creates it first, or the handoff prompt must be run from the old directory and perform Phase 0 itself.

**B. Where the single-file build ships.** Phase 3 option (c) said Pages should "compile down to a
single file at some point (maybe in releases?)". Reading it as: Pages serves the normal bundle now,
and the single file becomes a **release artifact**. If instead Pages should eventually serve the
single file itself, Phase 4 changes (deploy the inlined HTML as `index.html`).

**C. Attribution nuance.** `pi-agent@users.noreply.github.com` is not registered to any GitHub
account, so commits will display as an unlinked author named `pi-agent` rather than being attributed
to `RealBeepMcJeep`. That matches "specific to the pi agent"; say the word if you want them linked to
your account instead.
