import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { cn } from './cn';

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  children: ReactNode;
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
  size?: 'sm' | 'md' | 'lg';
}

export function Button({
  children,
  variant = 'secondary',
  size = 'md',
  className,
  ...props
}: ButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        'inline-flex select-none items-center justify-center gap-2 font-medium transition-[background-color,box-shadow,transform,opacity] duration-150 ease-(--ease-apple) active:scale-[0.97] disabled:pointer-events-none disabled:opacity-45',
        size === 'sm' && 'h-8 rounded-lg px-3 text-[13px]',
        size === 'md' && 'h-9 rounded-[10px] px-4 text-sm',
        size === 'lg' && 'h-11 rounded-xl px-6 text-[15px]',
        variant === 'primary' &&
          'bg-accent-600 text-white shadow-[0_1px_2px_rgba(30,27,75,0.25),inset_0_1px_0_rgba(255,255,255,0.15)] hover:bg-accent-700 dark:bg-accent-600 dark:hover:bg-accent-500',
        variant === 'secondary' &&
          'border border-black/[0.08] bg-white text-ink shadow-[0_1px_2px_rgba(0,0,0,0.05)] hover:bg-black/[0.03] dark:border-white/10 dark:bg-white/[0.08] dark:text-slate-100 dark:hover:bg-white/[0.12]',
        variant === 'ghost' &&
          'text-accent-700 hover:bg-accent-50 dark:text-accent-300 dark:hover:bg-white/[0.08]',
        variant === 'danger' &&
          'bg-red-600 text-white shadow-[0_1px_2px_rgba(127,29,29,0.3)] hover:bg-red-700 dark:bg-red-600 dark:hover:bg-red-700',
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}
