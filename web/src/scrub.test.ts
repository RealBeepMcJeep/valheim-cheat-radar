import { describe, expect, it } from 'vitest';
import { summarizeScrub, type ScrubAuditAction } from './scrub';

const row = (action: ScrubAuditAction['action'], prefab: string | null, item: string | null = null, contents = 0): ScrubAuditAction => ({ action, file: 'worlds_local/Dedicated/1.chunk', prefab, item, contents });

describe('summarizeScrub', () => {
  it('groups destroyed objects by kind and warns about their consequences', () => {
    const { lines, warnings } = summarizeScrub({
      mode: 'destroy',
      zdo_count_before: 10,
      zdo_count_after: 4,
      actions: [
        row('destroy', 'stone_wall_4x2'),
        row('destroy', 'stone_wall_4x2'),
        row('destroy', 'piece_chest', null, 3),
        row('destroy', 'portal_wood'),
        row('delete_item', 'Wood'),
        row('remove_from_container', 'piece_chest', 'Silver'),
      ],
    });
    expect(lines).toContainEqual({ label: 'Dropped items deleted', count: 1, examples: 'Wood' });
    expect(lines).toContainEqual({ label: 'Items removed from containers', count: 1, examples: 'Silver' });
    expect(lines).toContainEqual({ label: 'Destroyed: containers', count: 1, examples: 'piece_chest' });
    expect(lines).toContainEqual({ label: 'Destroyed: structures', count: 3, examples: 'stone_wall_4x2 ×2, portal_wood' });
    expect(warnings.join(' ')).toMatch(/1 portal/);
    expect(warnings.join(' ')).toMatch(/held 3 item/);
    expect(warnings.join(' ')).toMatch(/collapse/);
  });

  it('has nothing to warn about when flags are only cleared', () => {
    const { lines, warnings } = summarizeScrub({
      mode: 'clean',
      zdo_count_before: 2,
      zdo_count_after: 2,
      actions: [row('clear_flag', 'woodwall'), row('clear_flag', 'woodwall')],
    });
    expect(lines).toEqual([{ label: 'Flags cleared, everything kept', count: 2, examples: 'woodwall ×2' }]);
    expect(warnings).toEqual([]);
  });
});

describe('summarizeScrub with automatic backups', () => {
  it('counts only the live world and lists the backups changed alongside it', () => {
    const live = (action: ScrubAuditAction['action']) => ({ ...row(action, 'Wood'), file: 'worlds_local/Dedicated/1.chunk' });
    const copy = (action: ScrubAuditAction['action']) => ({ ...row(action, 'Wood'), file: 'worlds_local/Dedicated_backup_auto-20260927-121154/1.chunk' });
    const { lines } = summarizeScrub({
      mode: 'delete-items',
      zdo_count_before: 5,
      zdo_count_after: 4,
      auto_backups: ['Dedicated_backup_auto-20260927-121154'],
      actions: [live('delete_item'), copy('delete_item')],
    });
    expect(lines).toEqual([
      { label: 'Dropped items deleted', count: 1, examples: 'Wood' },
      { label: "Server's automatic backups changed the same way", count: 1, examples: 'Dedicated_backup_auto-20260927-121154' },
    ]);
  });
});
