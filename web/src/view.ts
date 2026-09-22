import type { Evidence, SortKey } from './types';

export function filterEvidence(rows: Evidence[], query: string, kind: string, status: string): Evidence[] {
  const needle = query.trim().toLocaleLowerCase();
  return rows.filter((row) => {
    if (kind !== 'all' && row.kind !== kind) return false;
    if (status !== 'all' && row.status !== status) return false;
    if (!needle) return true;
    return [
      row.archive,
      row.snapshot,
      row.first_seen_snapshot,
      row.last_seen_snapshot,
      row.source,
      row.internal_path,
      row.kind,
      row.owner_prefab_name,
      row.item_name,
      row.key_name,
      row.crafter_name,
      String(row.owner_prefab_hash),
      String(row.item_hash ?? ''),
      `${row.position.x} ${row.position.y} ${row.position.z}`,
    ]
      .filter(Boolean)
      .join(' ')
      .toLocaleLowerCase()
      .includes(needle);
  });
}

export function sortEvidence(rows: Evidence[], key: SortKey, direction: 'asc' | 'desc'): Evidence[] {
  const factor = direction === 'asc' ? 1 : -1;
  return [...rows].sort((left, right) => {
    const a = valueFor(left, key);
    const b = valueFor(right, key);
    if (typeof a === 'number' && typeof b === 'number') return (a - b) * factor;
    return String(a).localeCompare(String(b), undefined, { numeric: true }) * factor;
  });
}

function valueFor(row: Evidence, key: SortKey): string | number {
  switch (key) {
    case 'stack': return row.stack ?? -1;
    case 'x': return row.position.x;
    case 'y': return row.position.y;
    case 'z': return row.position.z;
    case 'owner_prefab_name': return row.owner_prefab_name ?? '';
    case 'item_name': return row.item_name ?? '';
    case 'snapshot': return row.snapshot;
    case 'kind': return row.kind;
    case 'status': return row.status;
  }
}

export function timelineSummary(archiveCount: number): string {
  return archiveCount < 2
    ? 'One save reports each logical occurrence as observed. It cannot establish a universal ZDO creation time or continuity; add multiple dated saves for approximate first/last-seen history.'
    : 'Sorted saves classify each stack-excluded occurrence relative to the latest save as new, persisted, or removed_or_cleared. First/last-seen labels are approximate because chunk records do not carry a universal ZDO creation timestamp.';
}
