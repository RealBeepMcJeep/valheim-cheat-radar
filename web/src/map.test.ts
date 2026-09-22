import { describe, expect, it } from 'vitest';
import { densityColor, peakDensity, statusFill } from './map';

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
    expect(densityColor(0, 0)).toBe('rgb(30,28,26)');
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

describe('statusFill', () => {
  it('gives every status its own colour', () => {
    const fills = ['new', 'persisted', 'removed_or_cleared', 'observed'].map(statusFill);
    expect(new Set(fills).size).toBe(4);
  });

  it('falls back to a neutral colour for an unknown status', () => {
    expect(statusFill('something-else')).toBe(statusFill('observed'));
  });
});
