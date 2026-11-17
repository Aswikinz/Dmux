import '@testing-library/jest-dom/vitest';
import { vi } from 'vitest';

// TanStack Virtual does not lay out rows in jsdom (its scroll element has zero
// size), so every virtualized list would render empty. Mock it to render all
// rows for unit tests.
vi.mock('@tanstack/react-virtual', () => {
  return {
    useVirtualizer: ({ count }: { count: number }) => {
      return {
        getVirtualItems: () =>
          Array.from({ length: count }, (_, index) => ({ index, start: index * 40 })),
        getTotalSize: () => count * 40,
        measureElement: undefined,
      };
    },
  };
});

// requestAnimationFrame is absent in jsdom. The theme auto-detects slow blur by
// sampling frames; a stub that always reports ~16ms frames keeps that effect
// from leaving stray DOM nodes behind.
if (typeof window !== 'undefined' && typeof window.requestAnimationFrame !== 'function') {
  window.requestAnimationFrame = (cb: FrameRequestCallback): number => {
    cb(Date.now());
    return 0;
  };
  window.cancelAnimationFrame = () => {};
}

// scrollIntoView is not implemented by jsdom; MappingQueue calls it when a
// queue item is selected.
if (typeof window !== 'undefined' && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

// jsdom does not implement matchMedia, which the theme code uses to detect the
// preferred colour scheme (see src/design/theme.tsx).
if (typeof window !== 'undefined' && typeof window.matchMedia !== 'function') {
  window.matchMedia = (query: string): MediaQueryList => {
    return {
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    } as MediaQueryList;
  };
}