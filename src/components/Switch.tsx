import { cn } from '../design/cn';

interface SwitchProps {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  className?: string;
}

/** An iOS-style toggle; the real control is a visually hidden checkbox. */
export function Switch({ label, checked, onChange, className }: SwitchProps) {
  return (
    <label
      className={cn(
        'inline-flex cursor-pointer select-none items-center gap-3 text-sm font-medium text-slate-700 dark:text-slate-300',
        className,
      )}
    >
      <span
        className={cn(
          'relative inline-flex h-[26px] w-[44px] shrink-0 items-center rounded-full transition-colors duration-200 ease-(--ease-apple)',
          checked ? 'bg-accent-600 dark:bg-accent-500' : 'bg-black/15 dark:bg-white/20',
        )}
      >
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          className="peer sr-only"
        />
        <span
          className={cn(
            'absolute left-[2px] h-[22px] w-[22px] rounded-full bg-white shadow-[0_2px_4px_rgba(0,0,0,0.2)] transition-transform duration-200 ease-(--ease-spring)',
            checked && 'translate-x-[18px]',
          )}
        />
        <span className="pointer-events-none absolute inset-0 rounded-full peer-focus-visible:ring-4 peer-focus-visible:ring-accent-500/30" />
      </span>
      {label}
    </label>
  );
}
