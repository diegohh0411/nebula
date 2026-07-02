import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { prefersReducedMotion } from './motion';

describe('prefersReducedMotion', () => {
  const matchMediaMock = vi.fn();

  beforeEach(() => {
    vi.stubGlobal('matchMedia', matchMediaMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('returns false when the user has not requested reduced motion', () => {
    matchMediaMock.mockReturnValue({ matches: false });
    expect(prefersReducedMotion()).toBe(false);
  });

  it('returns true when the user has requested reduced motion', () => {
    matchMediaMock.mockReturnValue({ matches: true });
    expect(prefersReducedMotion()).toBe(true);
  });

  it('queries the (prefers-reduced-motion: reduce) media feature', () => {
    matchMediaMock.mockReturnValue({ matches: false });
    prefersReducedMotion();
    expect(matchMediaMock).toHaveBeenCalledWith('(prefers-reduced-motion: reduce)');
  });

  it('returns false when matchMedia is unavailable on window', () => {
    const originalMatchMedia = window.matchMedia;
    delete (window as { matchMedia?: unknown }).matchMedia;
    try {
      expect(prefersReducedMotion()).toBe(false);
    } finally {
      (window as { matchMedia?: unknown }).matchMedia = originalMatchMedia;
    }
  });
});
