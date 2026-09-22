/** A density cell: [cellX, cellZ, zdoCount], in `cell_meters` units from the scanner. */
export type MapCell = [number, number, number];

/**
 * Log-scaled ember ramp. Bases hold thousands of ZDOs next to cells holding a
 * single one, so a linear ramp would render everything but the largest base black.
 */
export function densityColor(count: number, max: number): string {
  const t = max <= 1 ? 0 : Math.min(1, Math.log1p(Math.max(count, 0)) / Math.log1p(max));
  const red = Math.round(30 + t * 205);
  const green = Math.round(28 + t * 92);
  const blue = Math.round(26 + t * 24);
  return `rgb(${red},${green},${blue})`;
}

export function statusFill(status: string): string {
  switch (status) {
    case 'new': return '#ee8a48';
    case 'persisted': return '#8fbf7e';
    case 'removed_or_cleared': return '#b84a37';
    default: return '#5d9ac4';
  }
}

/** Largest single-cell ZDO count, used to normalise the colour ramp. */
export function peakDensity(cells: MapCell[]): number {
  return cells.reduce((max, [, , count]) => Math.max(max, count), 0);
}
