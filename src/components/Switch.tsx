import { cn } from '../design/cn';

interface SwitchProps {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  className?: string;
}

export function Switch({ label, checked, onChange, className }: SwitchProps) {
  return (
    <label
      className={cn(
        'inline-flex cursor-pointer items-center gap-3 text-sm font-medium text-slate-700 dark:text-slate-300',
        className,
      )}
    >
      <span className="relative inline-flex h-6 w-11 items-center rounded-full bg-slate-200 transition-colors dark:bg-slate-700">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          className="sr-only peer"
        />
        <span
          className={cn(
            'absolute left-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform peer-checked:translate-x-5',
          )}
        />
      </span>
      {label}
    </label>
  );
}
