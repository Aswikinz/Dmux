import type { ReactNode } from 'react';

interface ScreenHeaderProps {
  title: string;
  /** One plain sentence saying what this step is for. */
  subtitle?: string;
  /** Controls shown at the right of the title, such as a filter switch. */
  actions?: ReactNode;
}

/** The large title at the top of every step, in the macOS style. */
export function ScreenHeader({ title, subtitle, actions }: ScreenHeaderProps) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-4">
      <div>
        <h2 className="text-[28px] font-semibold leading-tight tracking-[-0.02em] text-ink dark:text-white">
          {title}
        </h2>
        {subtitle && (
          <p className="mt-1 max-w-2xl text-[15px] text-ink-secondary dark:text-slate-400">{subtitle}</p>
        )}
      </div>
      {actions}
    </div>
  );
}
