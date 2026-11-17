import type { ReactNode } from 'react';
import { cn } from './cn';
import { useTheme } from './theme';

interface PanelProps {
  children: ReactNode;
  className?: string;
}

/**
 * Glassmorphic panel. In reduced-effects mode it becomes a solid, bordered
 * panel with the same layout so the app still looks finished.
 */
export function Panel({ children, className }: PanelProps) {
  const { reducedEffects } = useTheme();
  return (
    <div
      className={cn(
        'rounded-xl border p-4 shadow-sm',
        reducedEffects
          ? 'border-slate-200 bg-white dark:border-slate-700 dark:bg-slate-900'
          : 'border-white/30 bg-white/80 backdrop-blur-[16px] dark:border-white/10 dark:bg-slate-900/70',
        className,
      )}
    >
      {children}
    </div>
  );
}
