import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { Button } from '../design/Button';
import { cn } from '../design/cn';
import { fieldClasses } from '../design/Input';

export interface PickerOption {
  /** What is sent back when the option is chosen (a bare name). */
  value: string;
  /** What the user reads. */
  label: string;
}

interface PairPickerProps {
  /** What is being paired, for the accessible name: "Counterpart for accounts". */
  label: string;
  options: PickerOption[];
  onPick: (value: string) => void;
  onCancel: () => void;
  className?: string;
}

/**
 * A searchable list of counterparts in the new database.  Type to narrow it;
 * the arrow keys move, Enter chooses, Escape closes.  Everything also works with
 * the mouse.  It is a combobox: focus stays in the text field and the highlighted
 * option is announced through `aria-activedescendant`.
 */
export function PairPicker({ label, options, onPick, onCancel, className }: PairPickerProps) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const baseId = useId();
  const listId = `${baseId}-list`;

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? options.filter((o) => o.label.toLowerCase().includes(q)) : options;
  }, [options, query]);

  // The picker opens because the user asked for it, so focus moves to it.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const current = Math.min(active, Math.max(0, shown.length - 1));

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    // Keys here belong to the picker, never to the list behind it.
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      e.stopPropagation();
      setActive(Math.min(current + 1, shown.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      e.stopPropagation();
      setActive(Math.max(current - 1, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      const choice = shown[current];
      if (choice) onPick(choice.value);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    } else {
      e.stopPropagation();
    }
  }

  return (
    <div className={cn('flex flex-col gap-2', className)} data-testid="pair-picker">
      <input
        ref={inputRef}
        type="text"
        role="combobox"
        aria-expanded="true"
        aria-controls={shown.length > 0 ? listId : undefined}
        aria-autocomplete="list"
        aria-activedescendant={shown[current] ? `${baseId}-opt-${current}` : undefined}
        aria-label={label}
        placeholder="Type to search the new database"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(0);
        }}
        onKeyDown={onKeyDown}
        className={fieldClasses}
      />
      {shown.length === 0 ? (
        <p role="status" className="px-1 text-sm text-slate-600 dark:text-slate-300">
          {options.length === 0
            ? 'Nothing in the new database is left to pair with.'
            : 'No match. Try fewer letters.'}
        </p>
      ) : (
        <ul
          id={listId}
          role="listbox"
          aria-label={`${label}: matching items`}
          className="max-h-48 overflow-auto rounded-lg border border-black/[0.08] bg-white dark:border-white/10 dark:bg-slate-900"
        >
          {shown.map((o, i) => (
            <li
              key={o.value}
              id={`${baseId}-opt-${i}`}
              role="option"
              aria-selected={i === current}
              onMouseEnter={() => setActive(i)}
              onClick={() => onPick(o.value)}
              className={cn(
                'cursor-pointer px-3 py-1.5 text-sm text-slate-800 dark:text-slate-100',
                i === current && 'bg-accent-50 dark:bg-accent-500/20',
              )}
            >
              {o.label}
            </li>
          ))}
        </ul>
      )}
      <div>
        <Button size="sm" variant="secondary" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
