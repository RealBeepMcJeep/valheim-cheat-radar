/** A density cell: [cellX, cellZ, zdoCount], in `cell_meters` units from the scanner. */
export type MapCell = [number, number, number];

type Rgb = [number, number, number];
type Stop = [number, Rgb];

/**
 * Multi-stop intensity ramp: indigo (near-empty) through blue, teal and amber to a hot red at the
 * densest cell. A single-hue ramp reads as "dark to bright"; this reads as "cold to hot", which is
 * easier to judge at a glance across the orders of magnitude a save's cell counts span.
 */
const DENSITY_STOPS: Stop[] = [
  [0, [18, 18, 26]],
  [0.25, [42, 62, 122]],
  [0.5, [48, 148, 132]],
  [0.75, [227, 176, 58]],
  [1, [233, 72, 52]],
];

function interpolateStops(t: number, stops: Stop[]): Rgb {
  const clamped = Math.min(1, Math.max(0, t));
  for (let index = 1; index < stops.length; index += 1) {
    const [t1, c1] = stops[index - 1];
    const [t2, c2] = stops[index];
    if (clamped <= t2) {
      const local = (clamped - t1) / (t2 - t1 || 1);
      return [0, 1, 2].map((channel) => Math.round(c1[channel] + (c2[channel] - c1[channel]) * local)) as Rgb;
    }
  }
  return stops[stops.length - 1][1];
}

/** Where a raw count sits on the log-scaled ramp, from 0 (empty) to 1 (the densest cell). */
export function densityScale(count: number, max: number): number {
  return max <= 1 ? 0 : Math.min(1, Math.log1p(Math.max(count, 0)) / Math.log1p(max));
}

/**
 * Log-scaled colour ramp. Bases hold thousands of ZDOs next to cells holding a single one, so a
 * linear ramp would render everything but the largest base the same dark colour.
 */
export function densityColor(count: number, max: number): string {
  const [red, green, blue] = interpolateStops(densityScale(count, max), DENSITY_STOPS);
  return `rgb(${red},${green},${blue})`;
}

/** CSS `linear-gradient` matching `densityColor`'s ramp exactly, for the legend swatch. */
export function densityGradientCss(): string {
  return `linear-gradient(to right, ${DENSITY_STOPS.map(([t, [r, g, b]]) => `rgb(${r},${g},${b}) ${Math.round(t * 100)}%`).join(', ')})`;
}

/** Evenly spaced legend ticks along the log ramp, as the raw counts they represent. */
export function densityTickCounts(peak: number, steps = 4): number[] {
  if (peak <= 1) return new Array(steps + 1).fill(0);
  const scale = Math.log1p(peak);
  return Array.from({ length: steps + 1 }, (_, index) => Math.round(Math.exp((index / steps) * scale) - 1));
}

/**
 * Biome fills, in the scanner's index order (see `BIOMES` in `src/lib.rs` and `FORMAT.md`).
 * Deliberately muted so evidence markers and the density shading stay readable over them.
 */
export const BIOME_COLORS = [
  '#6f8f4e', // meadows
  '#4c5f42', // swamp
  '#c9d2d6', // mountain
  '#2f4a2c', // blackforest
  '#c9b271', // plains
  '#6d3630', // ashlands
  '#bcd6e2', // deepnorth
  '#1f3d55', // ocean
  '#6c5b7d', // mistlands
] as const;

/** Colour for a biome index, with a neutral fallback for an index we do not know. */
export function biomeColor(index: number): string {
  return BIOME_COLORS[index] ?? '#3a3a38';
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

/** Mirrors `BIOME_MIN_WEIGHT`/`BIOME_MIN_SHARE_PERCENT` in `src/lib.rs`, for the popup's wording. */
export const BIOME_MIN_WEIGHT = 36;
export const BIOME_MIN_SHARE_PERCENT = 60;

/** One cell's raw `biome_detail` tuple, as sent by `browser_map_json`: see `types.ts`. */
export type BiomeDetailRow = [number, number, number, number, number, string, [number, number][]];

export type CellBiomeDetail = {
  total: number;
  decidedIndex: number;
  decidedShare: number;
  source: string;
  top: [number, number][];
};

/** Index `biome_detail` rows by cell so a click handler can look one up in O(1). */
export function indexBiomeDetail(rows: BiomeDetailRow[] | undefined): Map<string, CellBiomeDetail> {
  const index = new Map<string, CellBiomeDetail>();
  for (const [x, z, total, decidedIndex, decidedShare, source, top] of rows ?? []) {
    index.set(`${x},${z}`, { total, decidedIndex, decidedShare, source, top });
  }
  return index;
}

/**
 * The verdict text for a cell popup: the winning biome and its share, or — when there is none — why
 * not, mirroring the two ways `biome_verdict` in `src/lib.rs` can fail: not enough evidence, or no
 * biome with a clear majority of it.
 */
export function biomeVerdictText(detail: CellBiomeDetail | undefined, biomeNames: string[]): string {
  const total = detail?.total ?? 0;
  if (total < BIOME_MIN_WEIGHT) return `too little evidence: ${total} of ${BIOME_MIN_WEIGHT}`;
  if (!detail || detail.decidedIndex < 0) {
    const topWeight = detail?.top[0]?.[1] ?? 0;
    const topShare = Math.floor((topWeight * 100) / total);
    return `no clear winner: top share ${topShare}%`;
  }
  const name = biomeNames[detail.decidedIndex] ?? `biome ${detail.decidedIndex}`;
  return `${name} (${detail.decidedShare}% share)`;
}

/** Whether a decided verdict needed flora-hint votes, for cells where that happened. */
export function biomeSourceText(detail: CellBiomeDetail | undefined): string | null {
  if (!detail || detail.decidedIndex < 0) return null;
  return detail.source === 'hint'
    ? 'needed flora-hint votes (real game-tagged evidence alone was inconclusive)'
    : 'real game-tagged evidence alone';
}

export type PixelPoint<T> = { x: number; y: number; item: T };
export type PixelBucket<T> = { x: number; y: number; items: T[] };

/**
 * Grid-bucket points that are already in a shared pixel space (e.g. `map.project(latlng, zoom)`,
 * which is pan-invariant — only zoom moves a point between buckets, not panning). Each bucket's
 * centre is the running average of the points folded into it, so a badge sits among its markers
 * rather than at one arbitrary corner.
 *
 * ponytail: a point only ever joins the bucket its own coordinates fall into, so a marker near a
 * cell edge never merges with a denser neighbour one pixel across the line. Good enough for
 * decluttering a dense base; swap in a real spatial index if exact bucket boundaries ever matter.
 */
export function bucketByPixel<T>(points: PixelPoint<T>[], cellPx: number): PixelBucket<T>[] {
  const buckets = new Map<string, PixelBucket<T>>();
  for (const point of points) {
    const key = `${Math.floor(point.x / cellPx)},${Math.floor(point.y / cellPx)}`;
    const bucket = buckets.get(key);
    if (bucket) {
      const count = bucket.items.length + 1;
      bucket.x += (point.x - bucket.x) / count;
      bucket.y += (point.y - bucket.y) / count;
      bucket.items.push(point.item);
    } else {
      buckets.set(key, { x: point.x, y: point.y, items: [point.item] });
    }
  }
  return [...buckets.values()];
}

/** Higher means more urgent to review; ties fall back to the first status seen. */
const STATUS_SEVERITY: Record<string, number> = { persisted: 0, observed: 1, new: 2, removed_or_cleared: 3 };

/** The most severe status among a badge's records, to colour it like a single worst-case marker. */
export function mostSevereStatus(statuses: string[]): string {
  let worst = statuses[0] ?? 'observed';
  for (const status of statuses) {
    if ((STATUS_SEVERITY[status] ?? 0) > (STATUS_SEVERITY[worst] ?? 0)) worst = status;
  }
  return worst;
}
