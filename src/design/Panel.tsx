import type { ReactNode } from 'react';
import { cn } from './cn';
import { useTheme } from './theme';

interface PanelProps {
  children: ReactNode;
  className?: string;
}

/**
 * A grouped card in the macOS settings style: white, softly shadowed, with a
 * hairline border.  In reduced-effects mode the shadow and translucency are
 * dropped, so the layout is identical and still looks finished.
 */
export function Panel({ children, className }: PanelProps) {
  const { reducedEffects } = useTheme();
  return (
    <div
      className={cn(
        'rounded-2xl border p-5',
        reducedEffects
          ? 'border-black/10 bg-white dark:border-white/10 dark:bg-card-dark'
          : 'border-black/[0.06] bg-white/90 shadow-[0_1px_2px_rgba(0,0,0,0.04),0_8px_24px_-12px_rgba(30,27,75,0.12)] backdrop-blur-xl dark:border-white/[0.08] dark:bg-card-dark/90 dark:shadow-[0_1px_2px_rgba(0,0,0,0.4)]',
        className,
      )}
    >
      {children}
    </div>
  );
}
