import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react';
import type { ReactNode } from 'react';

type ThemeMode = 'light' | 'dark' | 'system';

interface ThemeContextValue {
  mode: ThemeMode;
  resolvedMode: 'light' | 'dark';
  reducedEffects: boolean;
  setMode: (mode: ThemeMode) => void;
  toggleReducedEffects: () => void;
}

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined);

const STORAGE_KEY = 'dmux-theme';
const REDUCED_KEY = 'dmux-reduced-effects';
const FRAME_TIME_THRESHOLD = 32; // ms; ~30fps sustained

function getInitialMode(): ThemeMode {
  try {
    return (localStorage.getItem(STORAGE_KEY) as ThemeMode | null) ?? 'system';
  } catch {
    return 'system';
  }
}

function getInitialReduced(): boolean {
  try {
    return localStorage.getItem(REDUCED_KEY) === 'true';
  } catch {
    return false;
  }
}

function resolveMode(mode: ThemeMode): 'light' | 'dark' {
  if (mode !== 'system') return mode;
  if (typeof window === 'undefined') return 'light';
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function measureBlurPerformance(): Promise<boolean> {
  return new Promise((resolve) => {
    if (typeof document === 'undefined' || typeof requestAnimationFrame === 'undefined') {
      resolve(false);
      return;
    }

    const el = document.createElement('div');
    el.style.cssText =
      'position:fixed;left:-9999px;top:0;width:100px;height:100px;backdrop-filter:blur(16px);';
    document.body.appendChild(el);

    let frames = 0;
    let slowFrames = 0;
    let last = performance.now();
    const duration = 500;
    const start = last;

    function tick(now: number) {
      frames += 1;
      if (now - last > FRAME_TIME_THRESHOLD) {
        slowFrames += 1;
      }
      last = now;

      if (now - start < duration) {
        requestAnimationFrame(tick);
      } else {
        document.body.removeChild(el);
        // Reduced effects if more than half the frames were slow.
        resolve(slowFrames / Math.max(frames, 1) > 0.5);
      }
    }

    requestAnimationFrame(tick);
  });
}

interface ThemeProviderProps {
  children: ReactNode;
}

export function ThemeProvider({ children }: ThemeProviderProps) {
  const [mode, setModeState] = useState<ThemeMode>(getInitialMode);
  const [reducedEffects, setReducedEffects] = useState<boolean>(getInitialReduced);
  const [autoReduced, setAutoReduced] = useState<boolean>(false);
  const resolvedMode = useMemo(() => resolveMode(mode), [mode]);

  useEffect(() => {
    document.documentElement.classList.toggle('dark', resolvedMode === 'dark');
  }, [resolvedMode]);

  useEffect(() => {
    const listener = (e: MediaQueryListEvent) => {
      if (mode === 'system') {
        document.documentElement.classList.toggle('dark', e.matches);
      }
    };
    const mql = window.matchMedia('(prefers-color-scheme: dark)');
    mql.addEventListener('change', listener);
    return () => mql.removeEventListener('change', listener);
  }, [mode]);

  useEffect(() => {
    if (getInitialReduced()) return;
    let mounted = true;
    measureBlurPerformance().then((slow) => {
      if (mounted && slow) {
        setAutoReduced(true);
      }
    });
    return () => {
      mounted = false;
    };
  }, []);

  const effectiveReduced = reducedEffects || autoReduced;

  const setMode = useCallback((next: ThemeMode) => {
    setModeState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // ignore storage errors
    }
  }, []);

  const toggleReducedEffects = useCallback(() => {
    setReducedEffects((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(REDUCED_KEY, String(next));
      } catch {
        // ignore
      }
      return next;
    });
  }, []);

  const value = useMemo(
    () => ({
      mode,
      resolvedMode,
      reducedEffects: effectiveReduced,
      setMode,
      toggleReducedEffects,
    }),
    [mode, resolvedMode, effectiveReduced, setMode, toggleReducedEffects],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error('useTheme must be used within a ThemeProvider');
  }
  return ctx;
}
