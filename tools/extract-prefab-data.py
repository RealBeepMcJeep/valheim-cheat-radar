#!/usr/bin/env python3
"""Extract Valheim prefab names and prefab -> biome assignments from the game's own data.

Why this exists: the map infers a biome per world cell from the objects a save actually contains,
which needs two lookup tables. Both are *game* data, never save data, so the generated files are
committed:

    prefab_names.txt    one prefab name per line (the scanner hashes each name the same way the game
                        does, so a save's prefab hashes can be resolved to names)
    prefab_biomes.txt   "<prefab name><TAB><biome>[,<biome>...]" sorted by prefab

Where the information lives (verified 2026-09-23; summarised in README.md, format notes in FORMAT.md):

  * Biome flags and their string names are code: `Heightmap.cs:16` (`public enum Biome`) and
    `Heightmap.cs:1366` (`BiomeToString`) in the decompiled assembly.
  * The assignments are serialized MonoBehaviour fields spread across the game's soft-reference
    bundles:
      - `ZoneSystem.m_vegetation` / `m_locations` / `m_clutter` (zone bundle, e.g. 17245031)
      - `SpawnSystemList.m_spawners` (`SpawnData.m_prefab` + `m_biome`)
      - `ClutterSystem.m_clutter` (`Clutter.m_prefab` + `m_biome`)
      - any prefab carrying a biome-tagged component (saplings, beehives, spawn areas, ...)
    They are read by walking every object and keeping biome bits found *anywhere* in a typetree,
    together with either an inline name field or the name of the object that owns the component.
    Because a name is always available next to the bits, no cross-bundle PPtr resolution is needed.

Usage:

    python tools/extract-prefab-data.py --bundles "<Valheim>/valheim_Data/StreamingAssets/SoftRef/Bundles" \
        --out-dir . --cache "%TEMP%/vcradar-prefab-cache"

Requires UnityPy (pip install UnityPy). Re-running is cheap: every bundle's extraction is cached.
"""

from __future__ import annotations

import argparse
import collections
import contextlib
import json
import re
import sys
from pathlib import Path

BIOME_BITS = {
    0x1: "meadows",
    0x2: "swamp",
    0x4: "mountain",
    0x8: "blackforest",
    0x10: "plains",
    0x20: "ashlands",
    0x40: "deepnorth",
    0x100: "ocean",
    0x200: "mistlands",
}

# Fields that can name the prefab a biome belongs to.
NAME_FIELDS = ("m_prefabName", "m_name", "m_Name", "m_prefab")
# Biome names leaking through as strings (e.g. a biome list member) are not prefabs.
BIOME_WORDS = {
    "black forest",
    "blackforest",
    "meadows",
    "swamp",
    "mountain",
    "plains",
    "ashlands",
    "deepnorth",
    "ocean",
    "mistlands",
    "none",
    "all",
    "land",
}
# Internal/system objects worth keeping: they are how a biome's spawn table is expressed.
KEEP_PREFIXES = ("_SpawnList", "SpawnArea", "SpawnSystem", "ZoneSystem", "ClutterSystem")
NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_.,()\- ]{1,63}$")


def read_tree(obj) -> dict | None:
    """Unity typetree for a MonoBehaviour, or None when the object carries no readable one."""
    try:
        tree = obj.read_typetree()
    except Exception:
        return None
    return tree if isinstance(tree, dict) else None


def read_name(obj) -> str | None:
    """Name of a GameObject, or None when it cannot be read."""
    try:
        name = obj.read().m_Name
    except Exception:
        return None
    return name or None


def cab_name(env) -> str | None:
    """Internal CAB name of a bundle, which is what another bundle's dependency list refers to."""
    for obj in env.objects:
        if obj.type.name == "AssetBundle":
            return getattr(obj.assets_file, "name", None)
    return None


def scene_references(env) -> tuple[list[str], list[tuple[int, int, int]]]:
    """Dependency names plus (file_id, path_id, biome bits) prefab references in a scene bundle.

    A scene's zone tables point at prefabs that live in *other* bundles, so these references are
    what carries the biome of trees and plants that no single prefab knows about.
    """
    dependencies: list[str] = []
    for obj in env.objects:
        if obj.type.name == "AssetBundle":
            read = obj.read()
            dependencies = [str(dep) for dep in (getattr(read, "m_Dependencies", None) or [])]
            break

    references: set[tuple[int, int, int]] = set()
    for obj in env.objects:
        if obj.type.name != "MonoBehaviour":
            continue
        tree = read_tree(obj)
        if tree is not None and has_biome(tree):
            collect_references(tree, 0, references)
    return dependencies, sorted(references)


def collect_references(node, bits: int, out: set[tuple[int, int, int]]) -> set[tuple[int, int, int]]:
    """Like `collect`, but keeps the (file, path) reference instead of a name."""
    if isinstance(node, dict):
        inherited = node.get("m_biome")
        if isinstance(inherited, int) and inherited:
            bits = inherited
        prefab = node.get("m_prefab")
        if bits and isinstance(prefab, dict) and prefab.get("m_PathID"):
            out.add((int(prefab.get("m_FileID", 0)), int(prefab["m_PathID"]), bits))
        for key, value in node.items():
            if key == "m_prefab":
                continue
            collect_references(value, bits, out)
    elif isinstance(node, list):
        for value in node:
            collect_references(value, bits, out)
    return out


def index_cabs(bundles: list[Path], cache: Path) -> dict[str, str]:
    """Map every bundle's internal CAB name to its file name (see `--index-cabs`)."""
    import UnityPy

    index: dict[str, str] = {}
    for count, bundle in enumerate(bundles, start=1):
        try:
            name = cab_name(UnityPy.load(str(bundle)))
        except Exception as cause:  # one broken bundle must not stop the index
            print(f"  ! {bundle.name}: {cause}", file=sys.stderr)
            continue
        if name:
            index[name.lower()] = bundle.name
        if count % 100 == 0:
            print(f"  {count}/{len(bundles)} bundles indexed", flush=True)
    (cache / "cab-index.json").write_text(json.dumps(index), encoding="utf-8")
    print(f"indexed {len(index)} cab names -> {cache / 'cab-index.json'}")
    return index


def resolve_scene(
    scene: Path, bundles: Path, cache: Path, out_dir: Path, table: dict[str, set[str]]
) -> None:
    """Add the biome of prefabs that a scene's zone tables reference (trees, plants, ore veins)."""
    import UnityPy

    index_file = cache / "cab-index.json"
    if not index_file.exists():
        print("run --index-cabs first", file=sys.stderr)
        raise SystemExit(2)
    cab_index = {name.lower(): file for name, file in json.loads(index_file.read_text(encoding="utf-8")).items()}

    env = UnityPy.load(str(scene))
    dependencies, references = scene_references(env)
    # `m_FileID` indexes the *serialized file's* external table, whose order is not the same as the
    # AssetBundle's dependency list (they disagree in practice), so read the file's own externals.
    first = next(iter(env.objects), None)
    externals = list(getattr(first.assets_file, "externals", None) or []) if first is not None else []
    print(
        f"scene {scene.name}: {len(references)} biome references; "
        f"{len(externals)} external files, {len(dependencies)} bundle dependencies"
    )

    # Group the references by the bundle that actually holds the prefab, so each loads once.
    by_bundle: dict[str, set[int]] = collections.defaultdict(set)
    bits_by_ref: dict[tuple[str, int], int] = collections.defaultdict(int)
    missing_files = 0
    for file_id, path_id, bits in references:
        if file_id == 0:
            bundle_file = scene.name
        else:
            bundle_file = None
            if 0 < file_id <= len(externals):
                match = re.search(r"(CAB-[0-9a-fA-F]+)", str(externals[file_id - 1]))
                if match:
                    bundle_file = cab_index.get(match.group(1).lower())
            if not bundle_file:
                missing_files += 1
                continue
        by_bundle[bundle_file].add(path_id)
        bits_by_ref[(bundle_file, path_id & 0xFFFFFFFFFFFFFFFF)] |= bits

    resolved: dict[tuple[str, int], str] = {}
    # Path ids are signed in the scene's typetree but may come back unsigned from a bundle, so compare
    # them masked to 64 bits rather than by Python value.
    for bundle_file, path_ids in by_bundle.items():
        try:
            holder = UnityPy.load(str(bundles / bundle_file))
        except Exception as cause:
            print(f"  ! {bundle_file}: {cause}", file=sys.stderr)
            continue
        wanted = {path_id & 0xFFFFFFFFFFFFFFFF for path_id in path_ids}
        for obj in holder.objects:
            if obj.type.name == "GameObject" and (obj.path_id & 0xFFFFFFFFFFFFFFFF) in wanted:
                name = read_name(obj)
                if name:
                    resolved[(bundle_file, obj.path_id & 0xFFFFFFFFFFFFFFFF)] = name
    print(f"resolved {len(resolved)}/{len(bits_by_ref)} referenced prefabs ({missing_files} refs had no bundle)")

    added = 0
    for (bundle_file, path_id), name in resolved.items():
        bits = bits_by_ref[(bundle_file, path_id)]
        cleaned = clean_name(name)
        if not cleaned or not bits:
            continue
        table.setdefault(cleaned, set()).update(
            biome for bit, biome in BIOME_BITS.items() if bits & bit
        )
        added += 1
    print(f"added or updated {added} prefabs from the scene's referenced tables")

    # Rewrite prefab_biomes.txt from the merged table.
    target = out_dir / "prefab_biomes.txt"
    target.write_text(
        "# <prefab name><TAB><biome>[,<biome>...] — extracted from the game's bundles by\n"
        "# tools/extract-prefab-data.py. Cells are coloured from these; see README.md.\n"
        + "".join(f"{name}\t{','.join(sorted(biomes))}\n" for name, biomes in sorted(table.items())),
        encoding="utf-8",
    )
    print(f"prefab_biomes.txt now holds {len(table)} tagged prefabs")


def has_biome(node) -> bool:
    if isinstance(node, dict):
        if isinstance(node.get("m_biome"), int) and node["m_biome"]:
            return True
        return any(has_biome(value) for value in node.values())
    if isinstance(node, list):
        return any(has_biome(value) for value in node)
    return False


def collect(node, bits: int, out: set[tuple[str, int]]) -> set[tuple[str, int]]:
    """Gather (name, biome bits) pairs anywhere below `node`, inheriting the nearest biome."""
    if isinstance(node, dict):
        inherited = node.get("m_biome")
        if isinstance(inherited, int) and inherited:
            bits = inherited
        if bits:
            for field in NAME_FIELDS:
                value = node.get(field)
                if isinstance(value, str) and value:
                    out.add((value, bits))
        for key, value in node.items():
            if key in ("m_prefab", "m_prefabName"):
                continue
            collect(value, bits, out)
    elif isinstance(node, list):
        for value in node:
            collect(value, bits, out)
    return out


def clean_name(name: str) -> str | None:
    name = name.strip().lstrip("$").strip()
    if not name or name.lower() in BIOME_WORDS:
        return None
    if not NAME_RE.match(name):
        return None
    if name.startswith("DEF-") or name.endswith(".prefab"):
        return None
    return name


def extract_bundle(path: Path) -> tuple[set[str], dict[str, set[str]]]:
    import UnityPy

    env = UnityPy.load(str(path))
    names: set[str] = set()
    biomes: dict[str, set[str]] = collections.defaultdict(set)

    def add(name: str | None, bits: int) -> None:
        cleaned = clean_name(name) if name else None
        if not cleaned:
            return
        names.add(cleaned)
        for bit, biome in BIOME_BITS.items():
            if bits & bit:
                biomes[cleaned].add(biome)

    pending: list[dict] = []
    for obj in env.objects:
        if obj.type.name == "MonoBehaviour":
            tree = read_tree(obj)
            if tree is not None and has_biome(tree):
                pending.append(tree)
        elif obj.type.name == "GameObject":
            name = read_name(obj)
            if name and clean_name(name):
                names.add(name)

    owner_ids = {
        tree["m_GameObject"]["m_PathID"]
        for tree in pending
        if isinstance(tree.get("m_GameObject"), dict) and tree["m_GameObject"].get("m_PathID")
    }
    owners: dict[int, str] = {}
    if owner_ids:
        for obj in env.objects:
            if obj.type.name == "GameObject" and obj.path_id in owner_ids:
                name = read_name(obj)
                if name:
                    owners[obj.path_id] = name

    for tree in pending:
        for name, bits in collect(tree, 0, set()):
            add(name, bits)
        holder = tree.get("m_GameObject")
        owner = None
        if isinstance(holder, dict):
            path_id = holder.get("m_PathID")
            if isinstance(path_id, int):
                owner = owners.get(path_id)
        if owner and owner.startswith(KEEP_PREFIXES):
            combined = 0
            for _, bits in collect(tree, 0, set()):
                combined |= bits
            if combined:
                add(owner, combined)

    return names, dict(biomes)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--bundles", required=True, help="directory of soft-reference bundles")
    parser.add_argument("--out-dir", default=".", help="where prefab_names.txt / prefab_biomes.txt go")
    parser.add_argument("--cache", default=None, help="per-bundle cache directory (makes reruns cheap)")
    parser.add_argument("--limit", type=int, default=0, help="only scan the first N bundles")
    parser.add_argument("--only", nargs="*", default=[], help="only scan these bundle file names")
    parser.add_argument(
        "--index-cabs",
        action="store_true",
        help="only build the cab-name index that --resolve-scene needs (no tables written)",
    )
    parser.add_argument(
        "--resolve-scene",
        metavar="BUNDLE",
        help="bundle holding Assets/Scenes/main.unity: resolve the prefab biomes its zone tables reference",
    )
    args = parser.parse_args()

    bundles = sorted(path for path in Path(args.bundles).iterdir() if path.is_file())
    if args.only:
        wanted = set(args.only)
        bundles = [path for path in bundles if path.name in wanted]
    elif args.limit:
        bundles = bundles[: args.limit]
    if not bundles:
        print("no bundles selected", file=sys.stderr)
        return 2

    cache = Path(args.cache) if args.cache else None
    if cache:
        cache.mkdir(parents=True, exist_ok=True)

    bundles_dir = Path(args.bundles)
    if args.index_cabs:
        if not cache:
            print("--index-cabs needs --cache", file=sys.stderr)
            return 2
        index_cabs(bundles, cache)
        return 0

    all_names: set[str] = set()
    all_biomes: dict[str, set[str]] = collections.defaultdict(set)
    scanned = cached = failed = 0
    for index, bundle in enumerate(bundles, start=1):
        entry = cache / f"{bundle.name}.json" if cache else None
        try:
            if entry and entry.exists():
                payload = json.loads(entry.read_text(encoding="utf-8"))
                cached += 1
            else:
                names, biomes = extract_bundle(bundle)
                payload = {"names": sorted(names), "biomes": {k: sorted(v) for k, v in biomes.items()}}
                if entry:
                    entry.write_text(json.dumps(payload), encoding="utf-8")
            all_names.update(payload["names"])
            for name, biome_list in payload["biomes"].items():
                all_biomes[name].update(biome_list)
            scanned += 1
        except Exception as cause:  # one broken bundle must not stop the sweep
            failed += 1
            print(f"  ! {bundle.name}: {cause}", file=sys.stderr)
        if index % 50 == 0:
            print(
                f"  {index}/{len(bundles)} bundles ({cached} cached, {failed} failed, "
                f"{len(all_biomes)} prefabs tagged)",
                flush=True,
            )

    tagged = {name: sorted(biome) for name, biome in all_biomes.items() if biome}
    out = Path(args.out_dir)
    with contextlib.suppress(OSError):
        out.mkdir(parents=True, exist_ok=True)
    (out / "prefab_names.txt").write_text(
        "# Valheim prefab names, extracted from the game's bundles by tools/extract-prefab-data.py\n"
        + "".join(f"{name}\n" for name in sorted(all_names)),
        encoding="utf-8",
    )
    (out / "prefab_biomes.txt").write_text(
        "# <prefab name><TAB><biome>[,<biome>...] — extracted from the game's bundles by\n"
        "# tools/extract-prefab-data.py. Cells are coloured from these; see README.md.\n"
        + "".join(f"{name}\t{','.join(biome)}\n" for name, biome in sorted(tagged.items())),
        encoding="utf-8",
    )
    print(
        f"\nbundles={len(bundles)} scanned={scanned} cached={cached} failed={failed}\n"
        f"names: {len(all_names)} -> prefab_names.txt\n"
        f"prefabs with biomes: {len(tagged)} -> prefab_biomes.txt\n"
        f"single-biome: {sum(1 for value in tagged.values() if len(value) == 1)}"
    )

    if args.resolve_scene:
        if not cache:
            print("--resolve-scene needs --cache", file=sys.stderr)
            return 2
        scene_table = {name: set(biomes) for name, biomes in all_biomes.items()}
        resolve_scene(bundles_dir / args.resolve_scene, bundles_dir, cache, out, scene_table)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
