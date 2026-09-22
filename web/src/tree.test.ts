import { describe, expect, it } from 'vitest';
import { CATEGORY_ORDER, breakdownLabel, classify, clusterEvidence, clusterSummaryLabel, evidenceLabel } from './tree';
import type { Evidence } from './types';

function row(overrides: Partial<Evidence> & { x?: number; y?: number; z?: number } = {}): Evidence {
  const { x = 0, y = 0, z = 0, ...rest } = overrides;
  return {
    status: 'observed',
    first_seen_snapshot: 's1',
    last_seen_snapshot: 's1',
    present_in_latest: true,
    occurrence_index: 1,
    archive: 'a.tar',
    snapshot: 's1',
    source: 'a.tar',
    internal_path: 'world/x.chunk',
    format: 'chunked_v41',
    chunk: 'x.chunk',
    chunk_version: 1,
    chunk_size: 1,
    chunk_revision: 1,
    zdo_ordinal: 1,
    owner_prefab_hash: 1,
    owner_prefab_name: 'Stone',
    position: { x, y, z },
    legacy_sector: null,
    key_hash: 1,
    key_name: 'k',
    kind: 'direct_item_data',
    item_hash: null,
    item_name: null,
    grid: null,
    quality: null,
    stack: null,
    variant: null,
    crafter_name: null,
    world_level: null,
    custom_data_keys: [],
    ...rest,
  } as Evidence;
}

describe('classify', () => {
  it('maps evidence kinds to categories', () => {
    expect(classify(row({ kind: 'container_inventory' }))).toBe('container');
    expect(classify(row({ kind: 'indexed_item_data' }))).toBe('containerItem');
    expect(classify(row({ kind: 'direct_item_data' }))).toBe('droppedItem');
    expect(classify(row({ kind: 'station_queued_cheated' }))).toBe('station');
  });

  it('refines catch-all zdo_cheated rows by prefab', () => {
    const zdo = (name: string) => classify(row({ kind: 'zdo_cheated', owner_prefab_name: name }));
    expect(zdo('stone_wall_2x1')).toBe('structure');
    expect(zdo('stone_pile')).toBe('structure');
    expect(zdo('portal_wood')).toBe('structure');
    expect(zdo('piece_chest_warderobe')).toBe('container');
    expect(zdo('piece_workbench')).toBe('station');
    expect(zdo('Troll')).toBe('mob');
    expect(zdo('VikingShip')).toBe('vehicle');
  });

  it('prefers containers over stations for chest-like prefabs', () => {
    // piece_chest must not be captured by the broader station/structure patterns.
    expect(classify(row({ kind: 'zdo_cheated', owner_prefab_name: 'piece_chest_blackmetal' }))).toBe('container');
  });

  it('falls back to Other for unknown prefabs instead of guessing', () => {
    expect(classify(row({ kind: 'zdo_cheated', owner_prefab_name: 'Mystery_Thing' }))).toBe('other');
    expect(classify(row({ kind: 'zdo_cheated', owner_prefab_name: null }))).toBe('other');
  });
});

describe('clusterEvidence', () => {
  it('groups nearby records and separates distant ones', () => {
    const clusters = clusterEvidence([row({ x: 0, z: 0 }), row({ x: 5, z: 0 }), row({ x: 500, z: 0 })], 30);
    expect(clusters).toHaveLength(2);
    expect(clusters[0].count).toBe(2);
    expect(clusters[1].count).toBe(1);
  });

  it('separates stacked records that share X/Z but differ in Y', () => {
    // The real-world case: items lost at ~5,100 m must not fold into the base below.
    const clusters = clusterEvidence([row({ x: -3539, y: 30, z: 964 }), row({ x: -3539, y: 5154, z: 964 })], 30);
    expect(clusters).toHaveLength(2);
    expect(clusters.every((cluster) => cluster.count === 1)).toBe(true);
  });

  it('merges more records as the radius grows', () => {
    const rows = [row({ x: 0, z: 0 }), row({ x: 40, z: 0 }), row({ x: 80, z: 0 })];
    expect(clusterEvidence(rows, 10)).toHaveLength(3);
    expect(clusterEvidence(rows, 50)).toHaveLength(2);
    expect(clusterEvidence(rows, 100)).toHaveLength(1);
  });

  it('documents the greedy chaining ceiling', () => {
    // Greedy nearest-centroid is not transitive: each record joins the nearest
    // centroid within `radius`, and that centroid then drifts. So a 50 m radius
    // does NOT merge records 40 m apart in a line once the centroid has moved.
    // Two records at 0 and 40 do merge at radius 50; adding an 80 m record does
    // not, because the 0+40 centroid sits at 20 and 80 is 60 m away.
    const pair = [row({ x: 0, z: 0 }), row({ x: 40, z: 0 })];
    expect(clusterEvidence(pair, 50)).toHaveLength(1);
    const triple = [...pair, row({ x: 80, z: 0 })];
    expect(clusterEvidence(triple, 50)).toHaveLength(2);
  });

  it('reports an averaged centroid for a merged cluster', () => {
    const clusters = clusterEvidence([row({ x: 0, y: 0, z: 0 }), row({ x: 10, y: 20, z: 0 })], 30);
    expect(clusters[0].centroid).toEqual({ x: 5, y: 10, z: 0 });
  });

  it('sorts clusters by size so the worst site comes first', () => {
    const clusters = clusterEvidence([row({ x: 0, z: 0 }), row({ x: 900, z: 0 }), row({ x: 901, z: 1 }), row({ x: 902, z: 2 })], 30);
    expect(clusters.map((cluster) => cluster.count)).toEqual([3, 1]);
    expect(clusters.map((cluster) => cluster.id)).toEqual([0, 1]);
  });

  it('groups categories in display order inside a cluster', () => {
    const clusters = clusterEvidence(
      [
        row({ x: 0, z: 0, kind: 'zdo_cheated', owner_prefab_name: 'stone_wall_2x1' }),
        row({ x: 1, z: 0, kind: 'container_inventory', owner_prefab_name: 'piece_chest' }),
        row({ x: 2, z: 0, kind: 'direct_item_data', owner_prefab_name: 'Wood' }),
      ],
      30,
    );
    expect(clusters[0].groups.map((group) => group.category)).toEqual(['container', 'droppedItem', 'structure']);
    const expected = CATEGORY_ORDER.filter((category) => ['container', 'droppedItem', 'structure'].includes(category));
    expect(clusters[0].groups.map((group) => group.category)).toEqual(expected);
  });

  it('keeps records with unusable coordinates visible instead of dropping them', () => {
    const clusters = clusterEvidence([row({ x: 0, z: 0 }), row({ x: Number.NaN, z: 0 })], 30);
    const unlocated = clusters.find((cluster) => !cluster.located);
    expect(unlocated?.count).toBe(1);
    expect(unlocated?.centroid).toBeNull();
  });
});

describe('breakdownLabel', () => {
  it('summarises a cluster with correct plurals, in display order', () => {
    const clusters = clusterEvidence(
      [
        row({ x: 0, z: 0, kind: 'container_inventory', owner_prefab_name: 'piece_chest' }),
        row({ x: 1, z: 0, kind: 'direct_item_data', owner_prefab_name: 'Wood' }),
        row({ x: 2, z: 0, kind: 'zdo_cheated', owner_prefab_name: 'stone_wall_2x1' }),
        row({ x: 3, z: 0, kind: 'zdo_cheated', owner_prefab_name: 'stone_pile' }),
      ],
      30,
    );
    // Order follows the group listing so the summary mirrors what expands below.
    expect(breakdownLabel(clusters[0].groups)).toBe('1 container, 1 dropped item, 2 structures');
  });

  it('uses singular wording for a lone record', () => {
    const clusters = clusterEvidence([row({ x: 0, z: 0, kind: 'zdo_cheated', owner_prefab_name: 'Troll' })], 30);
    expect(breakdownLabel(clusters[0].groups)).toBe('1 monster');
  });
});

describe('clusterSummaryLabel', () => {
  it('renders count then a colon then the breakdown', () => {
    const clusters = clusterEvidence(
      [
        row({ x: 0, z: 0, kind: 'zdo_cheated', owner_prefab_name: 'stone_wall_2x1' }),
        row({ x: 1, z: 0, kind: 'zdo_cheated', owner_prefab_name: 'stone_wall_2x1' }),
        row({ x: 2, z: 0, kind: 'zdo_cheated', owner_prefab_name: 'stone_pile' }),
        row({ x: 3, z: 0, kind: 'direct_item_data', owner_prefab_name: 'Wood' }),
        row({ x: 4, z: 0, kind: 'direct_item_data', owner_prefab_name: 'Stone' }),
        row({ x: 5, z: 0, kind: 'station_queued_cheated', owner_prefab_name: 'blastfurnace' }),
      ],
      30,
    );
    expect(clusterSummaryLabel(clusters[0])).toBe('6 records: 2 dropped items, 1 station, 3 structures');
  });

  it('omits the colon when there is nothing to break down', () => {
    expect(clusterSummaryLabel({ id: 0, centroid: null, located: false, count: 1, groups: [] })).toBe('1 record');
    expect(clusterSummaryLabel({ id: 1, centroid: null, located: false, count: 4, groups: [] })).toBe('4 records');
  });
});

describe('evidenceLabel', () => {
  it('prefers the item name, then the owner prefab', () => {
    expect(evidenceLabel(row({ item_name: 'Silver' }))).toBe('Silver');
    expect(evidenceLabel(row({ item_name: null, owner_prefab_name: 'stone_wall_2x1' }))).toBe('stone_wall_2x1');
  });
});
