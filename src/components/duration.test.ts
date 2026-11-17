import { describe, expect, it } from 'vitest';
import { formatDuration, formatDurationRange } from './duration';

describe('plain-language durations', () => {
  it('formats ranges in words', () => {
    expect(formatDurationRange(20, 40)).toBe('about 20–40 seconds');
    expect(formatDurationRange(60, 120)).toBe('about 1–2 minutes');
    expect(formatDurationRange(20, 100)).toBe('about 20 seconds to 2 minutes');
    expect(formatDurationRange(25, 200)).toBe('about 25 seconds to 3 minutes');
    expect(formatDurationRange(3600, 7200)).toBe('about 1–2 hours');
    expect(formatDurationRange(120, 120)).toBe('about 2 minutes');
    expect(formatDurationRange(0, 0)).toBe('not known yet');
    expect(formatDurationRange(Number.NaN, 5)).toBe('not known yet');
  });

  it('formats one duration', () => {
    expect(formatDuration(1)).toBe('about 1 second');
    expect(formatDuration(300)).toBe('about 5 minutes');
    expect(formatDuration(0)).toBe('under a second');
  });
});
