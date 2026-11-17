import { useState, useId } from 'react';
import type { ReactNode } from 'react';
import { cn } from './cn';

interface TooltipProps {
  children: ReactNode;
  label: string;
  explanation: string;
  className?: string;
}

/**
 * Plain-language explanation of a technical term, shown on hover/focus.
 */
export function Tooltip({ children, label, explanation, className }: TooltipProps) {
  const [open, setOpen] = useState(false);
  const id = useId();

  return (
    <span className={cn('relative inline-block', className)}>
      <button
        type="button"
        className="cursor-help underline decoration-dotted underline-offset-2"
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        aria-describedby={open ? id : undefined}
      >
        {children}
      </button>
      {open && (
        <span
          id={id}
          role="tooltip"
          className={cn(
            'absolute bottom-full left-1/2 z-50 mb-2 w-64 -translate-x-1/2 rounded-lg border p-3 text-sm shadow-lg',
            'border-slate-200 bg-white text-slate-800 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-100',
          )}
        >
          <strong className="mb-1 block">{label}</strong>
          {explanation}
        </span>
      )}
    </span>
  );
}
