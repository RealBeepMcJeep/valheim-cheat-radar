import { CATEGORY_LABELS, CATEGORY_ORDER, classifyPrefab } from './tree';

export type ScrubMode = 'clean' | 'delete-items' | 'destroy';

export const SCRUB_MODES: { mode: ScrubMode; label: string; detail: string }[] = [
  { mode: 'clean', label: 'Clean', detail: 'Clear every cheat flag. Every object and item stays.' },
  {
    mode: 'delete-items',
    label: 'Delete items',
    detail: 'Delete flagged items: dropped ones, entries in containers, items on stands. Every other flagged object stays, flag and all.',
  },
  {
    mode: 'destroy',
    label: 'Destroy',
    detail: 'Destroy every flagged object together with everything it holds, and delete flagged items anywhere else. Nothing flagged survives.',
  },
];

/** One row of the scanner's scrub audit (`scrub_audit_json` in src/lib.rs). */
export type ScrubAuditAction = {
  action: 'clear_flag' | 'delete_item' | 'remove_from_container' | 'empty_stand' | 'destroy';
  prefab: string | null;
  item: string | null;
  contents: number;
};
export type ScrubAudit = { mode: ScrubMode; zdo_count_before: number; zdo_count_after: number; actions: ScrubAuditAction[] };
export type ScrubSummaryLine = { label: string; count: number; examples: string };

/** "stone_wall_4x2 ×117, wood_floor ×72, +12 more" */
function examples(names: (string | null)[], limit = 4): string {
  const counts = new Map<string, number>();
  for (const name of names) counts.set(name ?? 'unknown', (counts.get(name ?? 'unknown') ?? 0) + 1);
  const sorted = [...counts].sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]));
  const shown = sorted.slice(0, limit).map(([name, count]) => (count > 1 ? `${name} ×${count}` : name));
  return sorted.length > limit ? `${shown.join(', ')}, +${sorted.length - limit} more` : shown.join(', ');
}

/** What a prepared scrub did, grouped for a human, plus the consequences worth a warning. */
export function summarizeScrub(audit: ScrubAudit): { lines: ScrubSummaryLine[]; warnings: string[] } {
  const of = (action: ScrubAuditAction['action']) => audit.actions.filter((row) => row.action === action);
  const lines: ScrubSummaryLine[] = [];
  const add = (label: string, rows: ScrubAuditAction[], name: (row: ScrubAuditAction) => string | null) => {
    if (rows.length) lines.push({ label, count: rows.length, examples: examples(rows.map(name)) });
  };
  add('Flags cleared, everything kept', of('clear_flag'), (row) => row.prefab);
  add('Dropped items deleted', of('delete_item'), (row) => row.prefab);
  add('Items removed from containers', of('remove_from_container'), (row) => row.item);
  add('Items taken off stands', of('empty_stand'), (row) => row.item);
  const destroyed = of('destroy');
  for (const category of CATEGORY_ORDER) {
    add(`Destroyed: ${CATEGORY_LABELS[category].toLowerCase()}`, destroyed.filter((row) => classifyPrefab(row.prefab ?? '') === category), (row) => row.prefab);
  }

  const warnings: string[] = [];
  const count = (rows: ScrubAuditAction[], test: (row: ScrubAuditAction) => boolean) => rows.filter(test).length;
  const portals = count(destroyed, (row) => /portal/i.test(row.prefab ?? ''));
  if (portals) warnings.push(`${portals} portal(s) destroyed: each one's partner is left with nothing to connect to.`);
  const holders = destroyed.filter((row) => row.contents > 0);
  if (holders.length) {
    const held = holders.reduce((sum, row) => sum + row.contents, 0);
    warnings.push(`${holders.length} destroyed object(s) held ${held} item(s), flagged or not, which go with them.`);
  }
  if (count(destroyed, (row) => classifyPrefab(row.prefab ?? '') === 'structure')) {
    warnings.push('Destroying building pieces can leave others without support; those may collapse when the area next loads.');
  }
  if (count(destroyed, (row) => classifyPrefab(row.prefab ?? '') === 'mob')) {
    warnings.push('Destroyed creatures are gone for good, tamed ones included.');
  }
  if (count(destroyed, (row) => classifyPrefab(row.prefab ?? '') === 'station')) {
    warnings.push('Destroyed crafting stations take anything queued in them along.');
  }
  return { lines, warnings };
}
