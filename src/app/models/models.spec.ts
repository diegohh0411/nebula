import { formatEta } from './models';

describe('formatEta', () => {
  it('formats sub-minute durations as seconds', () => {
    expect(formatEta(30)).toBe('~30s left');
  });

  it('formats durations under an hour as minutes', () => {
    expect(formatEta(12 * 60)).toBe('~12 min left');
  });

  it('formats multi-hour durations with minutes', () => {
    expect(formatEta(2 * 3600 + 10 * 60)).toBe('~2h 10m left');
  });

  it('omits minutes for exact hours', () => {
    expect(formatEta(2 * 3600)).toBe('~2h left');
  });

  it('returns empty string for zero, negative, or non-finite input', () => {
    expect(formatEta(0)).toBe('');
    expect(formatEta(-5)).toBe('');
    expect(formatEta(Infinity)).toBe('');
    expect(formatEta(NaN)).toBe('');
  });
});
