import type { Evidence } from './types';

/**
 * Evidence taxonomy for the location tree.
 *
 * `kind` tells us the nature of the record, but `zdo_cheated` is a catch-all:
 * a flagged ZDO can be a wall, a chest, a workbench, a creature, or a boat. So
 * prefab names refine those rows into something a human can act on.
 */
export type Category =
  | 'container'
  | 'containerItem'
  | 'droppedItem'
  | 'station'
  | 'structure'
  | 'mob'
  | 'vehicle'
  | 'other';

export const CATEGORY_LABELS: Record<Category, string> = {
  container: 'Containers',
  containerItem: 'Items inside containers',
  droppedItem: 'Dropped items',
  station: 'Workbenches & stations',
  structure: 'Structures',
  mob: 'Monsters',
  vehicle: 'Vehicles',
  other: 'Other',
};

/** Display order within a cluster: things with contents first, scenery last. */
export const CATEGORY_ORDER: Category[] = [
  'container',
  'containerItem',
  'droppedItem',
  'station',
  'structure',
  'mob',
  'vehicle',
  'other',
];

/** Singular forms for compact summaries, e.g. "1 container, 7 structures". */
export const CATEGORY_SINGULAR: Record<Category, string> = {
  container: 'container',
  containerItem: 'container item',
  droppedItem: 'dropped item',
  station: 'station',
  structure: 'structure',
  mob: 'monster',
  vehicle: 'vehicle',
  other: 'other',
};

/** "1 container, 7 structures, 2 dropped items" — ordered, singular-aware. */
export function breakdownLabel(groups: TreeGroup[]): string {
  return groups
    .map((group) => `${group.rows.length} ${CATEGORY_SINGULAR[group.category]}${group.rows.length === 1 ? '' : 's'}`)
    .join(', ');
}

/** "29 records: 2 dropped items, 1 station, 26 structures" */
export function clusterSummaryLabel(cluster: TreeCluster): string {
  const noun = cluster.count === 1 ? 'record' : 'records';
  const breakdown = breakdownLabel(cluster.groups);
  return breakdown ? `${cluster.count} ${noun}: ${breakdown}` : `${cluster.count} ${noun}`;
}

const CONTAINER_PREFAB = /^piece_chest|^piece_wardrobe|^piece_chest_warderobe/i;
const STATION_PREFAB =
  /^piece_workbench|^piece_forge|^piece_cauldron|^piece_stonecutter|^piece_artisanstation|^piece_oven|^piece_fermenter|^piece_spinningwheel|^piece_windmill|^piece_blackforge|^piece_galdr|^piece_eitr|^piece_preptable|^piece_magetable|^piece_cookingstation|^piece_sapcollector|^piece_beehive|^blastfurnace|^charcoal_kiln|^smelter|^windmill|^beehive/i;
const VEHICLE_PREFAB = /^(VikingShip|Karve|Raft|Cart|Cart_item|Sledge|CargoCrate|DvergerCart)$/;
const STRUCTURE_PREFAB =
  /wall|floor|roof|gate|door|stair|beam|pole|pillar|arch|portal|window|fence|bridge|ladder|bed|throne|banner|table|chair|torch|brazier|pile|log|sign|rug|armorstand|itemstand|trap|spike|deck|hull|mast|dragon|tower|palisade|stone|wood|stake/i;

/**
 * Curated creature prefabs. Deliberately a list rather than a pattern: guessing
 * a creature from a name shape would misfile build pieces, and an honest
 * "Other" bucket is better than a confident wrong group.
 */
const MOB_PREFABS = new Set([
  'Troll', 'Troll_log', 'Greydwarf', 'GreydwarfBrute', 'GreydwarfShaman', 'Greyling',
  'Boar', 'Boar_piggy', 'Neck', 'Wolf', 'Wolf_cub', 'Fenring', 'Fenring_cultist',
  'Draugr', 'Draugr_elite', 'Skeleton', 'Skeleton_Poison', 'Skeleton_Hildir',
  'Skeleton_Friendly', 'Goblin', 'GoblinBrute', 'GoblinShaman', 'Blob', 'BlobTar',
  'BlobLava', 'Ghost', 'Bat', 'Hatchling', 'Drake', 'Serpent', 'Lox', 'Lox_Calf',
  'Deathsquito', 'Leech', 'Surtling', 'Wraith', 'Seeker', 'SeekerBrood', 'SeekerBrute',
  'Tick', 'Gjall', 'Dverger', 'DvergerArcher', 'DvergerMage', 'DvergerMageFire',
  'DvergerMageIce', 'DvergerMageSupport', 'Charred', 'CharredArcher', 'CharredMage',
  'Charred_Twitcher', 'Morgen', 'BonemawSerpent', 'Fader', 'UnstableLavaRock',
  'TrainingDummy', 'WoodenDummy', 'Skeleton_NoArcher',
]);

export function classify(row: Evidence): Category {
  switch (row.kind) {
    case 'container_inventory': return 'container';
    case 'indexed_item_data': return 'containerItem';
    case 'direct_item_data': return 'droppedItem';
    case 'station_queued_cheated': return 'station';
    default: break;
  }
  const name = row.owner_prefab_name ?? '';
  if (!name) return 'other';
  if (CONTAINER_PREFAB.test(name)) return 'container';
  if (STATION_PREFAB.test(name)) return 'station';
  if (MOB_PREFABS.has(name)) return 'mob';
  if (VEHICLE_PREFAB.test(name)) return 'vehicle';
  if (STRUCTURE_PREFAB.test(name)) return 'structure';
  return 'other';
}

export type Centroid = { x: number; y: number; z: number };

export type TreeGroup = { category: Category; label: string; rows: Evidence[] };

export type TreeCluster = {
  id: number;
  /** Null only for the synthetic bucket holding records with unusable coordinates. */
  centroid: Centroid | null;
  located: boolean;
  count: number;
  groups: TreeGroup[];
};

export const CLUSTER_RADIUS_OPTIONS = [10, 30, 50, 100];
export const DEFAULT_CLUSTER_RADIUS = 30;

function byNameThenPosition(left: Evidence, right: Evidence): number {
  const a = left.item_name ?? left.owner_prefab_name ?? left.kind;
  const b = right.item_name ?? right.owner_prefab_name ?? right.kind;
  return (
    a.localeCompare(b, undefined, { numeric: true }) ||
    left.position.y - right.position.y ||
    left.position.x - right.position.x ||
    left.position.z - right.position.z
  );
}

export function evidenceLabel(row: Evidence): string {
  return row.item_name ?? row.owner_prefab_name ?? row.kind;
}

/** Bucket rows by category in display order, sorted within each group. */
function groupByCategory(rows: Evidence[]): TreeGroup[] {
  const byCategory = new Map<Category, Evidence[]>();
  for (const row of rows) {
    const category = classify(row);
    const list = byCategory.get(category);
    if (list) list.push(row);
    else byCategory.set(category, [row]);
  }
  return CATEGORY_ORDER.flatMap((category) => {
    const grouped = byCategory.get(category);
    if (!grouped) return [];
    return [{ category, label: CATEGORY_LABELS[category], rows: [...grouped].sort(byNameThenPosition) }];
  });
}

/**
 * Greedy nearest-centroid grouping in full 3D.
 *
 * 3D rather than X/Z alone: Valheim saves contain items lost at extreme
 * altitude (observed ~5,100 m above a Y≈30 m world), and an X/Z-only match
 * would quietly fold those into whatever base sits underneath them.
 *
 * ponytail: greedy chaining is not transitive — a record joins the nearest
 * centroid within `radius`, and that centroid then drifts, so a 50 m radius can
 * split a 80 m line of drops into two clusters. Good enough for site-level
 * triage; swap in DBSCAN/union-find if exact cluster boundaries ever matter.
 */
export function clusterEvidence(rows: Evidence[], radius: number): TreeCluster[] {
  const centroids: Centroid[] = [];
  const buckets: Evidence[][] = [];
  const unlocated: Evidence[] = [];

  for (const row of rows) {
    const { x, y, z } = row.position;
    if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) {
      unlocated.push(row);
      continue;
    }
    let best = -1;
    let bestDistance = Infinity;
    for (let index = 0; index < centroids.length; index += 1) {
      const centroid = centroids[index];
      const distance = (x - centroid.x) ** 2 + (y - centroid.y) ** 2 + (z - centroid.z) ** 2;
      if (distance <= radius * radius && distance < bestDistance) {
        best = index;
        bestDistance = distance;
      }
    }
    if (best === -1) {
      centroids.push({ x, y, z });
      buckets.push([row]);
    } else {
      buckets[best].push(row);
      const count = buckets[best].length;
      const centroid = centroids[best];
      centroids[best] = {
        x: centroid.x + (x - centroid.x) / count,
        y: centroid.y + (y - centroid.y) / count,
        z: centroid.z + (z - centroid.z) / count,
      };
    }
  }

  const clusters: TreeCluster[] = buckets.map((bucket, index) => ({
    id: index,
    centroid: centroids[index],
    located: true,
    count: bucket.length,
    groups: groupByCategory(bucket),
  }));

  // Biggest site first: that is where cleanup effort pays off.
  clusters.sort((left, right) => right.count - left.count || left.id - right.id);
  clusters.forEach((cluster, index) => { cluster.id = index; });

  if (unlocated.length) {
    clusters.push({
      id: clusters.length,
      centroid: null,
      located: false,
      count: unlocated.length,
      groups: groupByCategory(unlocated),
    });
  }

  return clusters;
}
