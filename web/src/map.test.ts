import { describe, expect, it } from 'vitest';
import {
  BIOME_COLORS,
  biomeColor,
  biomeSourceText,
  biomeVerdictText,
  bucketByPixel,
  densityColor,
  densityGradientCss,
  densityTickCounts,
  indexBiomeDetail,
  mostSevereStatus,
  peakDensity,
  statusFill,
} from './map';
import type { BiomeDetailRow } from './map';

describe('biomeColor', () => {
  it('maps each biome index to a distinct colour and falls back for unknown indices', () => {
    expect(new Set(BIOME_COLORS).size).toBe(BIOME_COLORS.length);
    expect(biomeColor(0)).toBe(BIOME_COLORS[0]);
    expect(biomeColor(BIOME_COLORS.length - 1)).toBe(BIOME_COLORS[BIOME_COLORS.length - 1]);
    expect(biomeColor(99)).toBe('#3a3a38');
  });
});

describe('densityColor', () => {
  it('darkens sparse cells and brightens dense ones', () => {
    const sparse = densityColor(1, 2000);
    const dense = densityColor(2000, 2000);
    expect(sparse).not.toBe(dense);
    const red = (rgb: string) => Number(rgb.match(/\d+/g)![0]);
    expect(red(dense)).toBeGreaterThan(red(sparse));
  });

  it('stays in a valid rgb range at the extremes', () => {
    for (const [count, max] of [[0, 0], [0, 10], [10, 10], [99999, 10]] as const) {
      const channels = densityColor(count, max).match(/\d+/g)!.map(Number);
      expect(channels).toHaveLength(3);
      for (const channel of channels) {
        expect(channel).toBeGreaterThanOrEqual(0);
        expect(channel).toBeLessThanOrEqual(255);
      }
    }
  });

  it('handles an all-empty grid without producing NaN', () => {
    expect(densityColor(0, 0)).toBe('rgb(18,18,26)');
  });
});

describe('densityGradientCss', () => {
  it('produces a CSS linear-gradient with stops from 0% to 100%', () => {
    const css = densityGradientCss();
    expect(css).toMatch(/^linear-gradient\(to right, .*0%.*100%\)$/);
  });
});

describe('densityTickCounts', () => {
  it('starts at zero and ends at the peak', () => {
    const ticks = densityTickCounts(1000, 4);
    expect(ticks).toHaveLength(5);
    expect(ticks[0]).toBe(0);
    expect(ticks[4]).toBe(1000);
    // Monotonically increasing.
    for (let i = 1; i < ticks.length; i += 1) expect(ticks[i]).toBeGreaterThanOrEqual(ticks[i - 1]);
  });

  it('is all zero for an empty or single-count grid', () => {
    expect(densityTickCounts(0)).toEqual([0, 0, 0, 0, 0]);
    expect(densityTickCounts(1)).toEqual([0, 0, 0, 0, 0]);
  });
});

describe('peakDensity', () => {
  it('finds the largest cell count', () => {
    expect(peakDensity([[0, 0, 4], [1, 1, 99], [2, 2, 7]])).toBe(99);
  });

  it('returns zero for an empty grid', () => {
    expect(peakDensity([])).toBe(0);
  });
});

describe('indexBiomeDetail / biomeVerdictText / biomeSourceText', () => {
  const names = ['meadows', 'swamp', 'mountain'];
  const rows: BiomeDetailRow[] = [
    [0, 0, 36, 1, 100, 'real', [[1, 36]]],
    [1, 1, 48, 2, 100, 'hint', [[2, 48]]],
    [2, 2, 24, -1, 0, 'none', [[0, 24]]],
    [3, 3, 48, -1, 0, 'none', [[0, 24], [2, 24]]],
  ];
  const index = indexBiomeDetail(rows);

  it('indexes rows by cell', () => {
    expect(index.size).toBe(4);
    expect(index.get('0,0')?.decidedIndex).toBe(1);
  });

  it('reports the winning biome and its share when there is a verdict', () => {
    expect(biomeVerdictText(index.get('0,0'), names)).toBe('swamp (100% share)');
    expect(biomeVerdictText(index.get('1,1'), names)).toBe('mountain (100% share)');
  });

  it('explains too little evidence below the weight bar', () => {
    expect(biomeVerdictText(index.get('2,2'), names)).toBe('too little evidence: 24 of 36');
  });

  it('explains a split vote once the weight bar is cleared', () => {
    expect(biomeVerdictText(index.get('3,3'), names)).toBe('no clear winner: top share 50%');
  });

  it('treats a missing cell the same as zero evidence', () => {
    expect(biomeVerdictText(undefined, names)).toBe('too little evidence: 0 of 36');
  });

  it('says whether a verdict needed flora-hint votes', () => {
    expect(biomeSourceText(index.get('0,0'))).toBe('real game-tagged evidence alone');
    expect(biomeSourceText(index.get('1,1'))).toContain('flora-hint');
    expect(biomeSourceText(index.get('2,2'))).toBeNull();
  });
});

describe('bucketByPixel', () => {
  it('merges points within the same cell and centres the bucket among them', () => {
    const buckets = bucketByPixel(
      [{ x: 1, y: 1, item: 'a' }, { x: 3, y: 2, item: 'b' }, { x: 100, y: 100, item: 'c' }],
      10,
    );
    expect(buckets).toHaveLength(2);
    const merged = buckets.find((bucket) => bucket.items.length === 2)!;
    expect(merged.items.sort()).toEqual(['a', 'b']);
    expect(merged.x).toBe(2);
    expect(merged.y).toBe(1.5);
    const solo = buckets.find((bucket) => bucket.items.length === 1)!;
    expect(solo.items).toEqual(['c']);
  });

  it('keeps points in separate buckets once they cross a cell boundary', () => {
    const buckets = bucketByPixel([{ x: 0, y: 0, item: 1 }, { x: 20, y: 0, item: 2 }], 10);
    expect(buckets).toHaveLength(2);
  });
});

describe('mostSevereStatus', () => {
  it('ranks removed_or_cleared above new above observed above persisted', () => {
    expect(mostSevereStatus(['persisted', 'new'])).toBe('new');
    expect(mostSevereStatus(['new', 'removed_or_cleared'])).toBe('removed_or_cleared');
    expect(mostSevereStatus(['observed', 'persisted'])).toBe('observed');
  });

  it('falls back to the first status for an unknown one', () => {
    expect(mostSevereStatus(['mystery'])).toBe('mystery');
  });
});

describe('statusFill', () => {
  it('gives every status its own colour', () => {
    const fills = ['new', 'persisted', 'removed_or_cleared', 'observed'].map(statusFill);
    expect(new Set(fills).size).toBe(4);
  });

  it('falls back to a neutral colour for an unknown status', () => {
    expect(statusFill('something-else')).toBe(statusFill('observed'));
  });
});
