import type { InputHTMLAttributes } from 'react';
import { cn } from './cn';

interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
}

/** The shared look of text fields and selects (Apple-style focus ring). */
export const fieldClasses =
  'h-9 w-full rounded-[10px] border border-black/[0.1] bg-white px-3 text-sm text-ink shadow-[inset_0_1px_1px_rgba(0,0,0,0.03)] transition-[border-color,box-shadow] duration-150 placeholder:text-slate-400 focus:border-accent-500 focus:outline-none focus:ring-4 focus:ring-accent-500/20 dark:border-white/10 dark:bg-white/[0.06] dark:text-slate-100 dark:focus:border-accent-400';

export function Input({ label, className, id, ...props }: InputProps) {
  const inputId = id ?? (label ? `field-${label.replace(/\s+/g, '-').toLowerCase()}` : undefined);
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      {label && (
        <label htmlFor={inputId} className="text-[13px] font-medium text-slate-700 dark:text-slate-300">
          {label}
        </label>
      )}
      <input id={inputId} className={fieldClasses} {...props} />
    </div>
  );
}
