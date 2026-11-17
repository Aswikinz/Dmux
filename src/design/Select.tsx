import type { SelectHTMLAttributes } from 'react';
import { ChevronsUpDown } from 'lucide-react';
import { cn } from './cn';
import { fieldClasses } from './Input';

interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  options: { value: string; label: string }[];
}

export function Select({ label, options, className, id, ...props }: SelectProps) {
  const selectId = id ?? (label ? `field-${label.replace(/\s+/g, '-').toLowerCase()}` : undefined);
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      {label && (
        <label htmlFor={selectId} className="text-[13px] font-medium text-slate-700 dark:text-slate-300">
          {label}
        </label>
      )}
      <div className="relative">
        <select id={selectId} className={cn(fieldClasses, 'dmux-select cursor-pointer pr-8')} {...props}>
          {options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
        <ChevronsUpDown
          className="pointer-events-none absolute right-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-400"
          aria-hidden="true"
        />
      </div>
    </div>
  );
}
