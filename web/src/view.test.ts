import { describe, expect, it } from 'vitest';
import { filterEvidence, sortEvidence, timelineSummary } from './view';
import type { Evidence } from './types';

const row = (overrides: Partial<Evidence> = {}): Evidence => ({
  status: 'new', first_seen_snapshot: '2024-01-02', last_seen_snapshot: '2024-01-02', present_in_latest: true, occurrence_index: 1, archive: 'b.tar.zst', snapshot: '2024-01-02', source: 'b.tar.zst', internal_path: 'world/a.chunk', format: 'chunked_v41', chunk: 'a.chunk', chunk_version: 41, chunk_size: 1, chunk_revision: 2, zdo_ordinal: 1, owner_prefab_hash: 10, owner_prefab_name: 'piece_chest', position: { x: 4, y: 2, z: 1 }, legacy_sector: null, key_hash: 20, key_name: 'itemData', kind: 'direct_item_data', item_hash: 30, item_name: 'Silver', grid: { x: 0, y: 1 }, quality: 1, stack: 5, variant: 0, crafter_name: null, world_level: 0, custom_data_keys: [], ...overrides,
});

describe('evidence views', () => {
  it('filters by text, kind, and status', () => {
    const rows = [row(), row({ kind: 'zdo_cheated', status: 'persisted', owner_prefab_name: 'ward', item_name: 'Iron' })];
    expect(filterEvidence(rows, 'silver', 'all', 'all')).toHaveLength(1);
    expect(filterEvidence(rows, '', 'zdo_cheated', 'persisted')).toHaveLength(1);
    expect(filterEvidence(rows, 'missing', 'all', 'all')).toHaveLength(0);
  });

  it('sorts without mutating the source array', () => {
    const rows = [row({ stack: 2 }), row({ stack: 9 })];
    expect(sortEvidence(rows, 'stack', 'desc').map((item) => item.stack)).toEqual([9, 2]);
    expect(rows.map((item) => item.stack)).toEqual([2, 9]);
  });

  it('explains the single-save timeline limitation', () => {
    expect(timelineSummary(1)).toContain('One save');
    expect(timelineSummary(2)).toContain('Sorted saves');
  });
});
