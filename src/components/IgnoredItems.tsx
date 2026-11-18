import { useId, useState } from 'react';
import { Button } from '../design/Button';
import type { IgnoreItem } from '../api/contract';

/**
 * The ignored tables and columns, each with its required reason.  The count is
 * always shown; the list itself opens on request, so a run that left out many
 * tables (for example "not selected for this run") does not bury the verdict.
 * Every exported report still lists each item on its first page.
 */
export function IgnoredItems({ ignored }: { ignored: IgnoreItem[] }) {
  const [open, setOpen] = useState(false);
  const listId = useId();
  if (ignored.length === 0) {
    return null;
  }
  const count = ignored.length === 1 ? '1 item' : `${ignored.length} items`;
  return (
    <div className="mt-3" data-testid="ignored-items">
      <div className="flex flex-wrap items-center gap-3">
        <h3 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">
          Ignored
        </h3>
        <span className="text-sm text-slate-700 dark:text-slate-300">
          {count}, each with its reason
        </span>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          aria-controls={listId}
        >
          {open ? 'Hide ignored items' : 'Show ignored items'}
        </Button>
      </div>
      {open && (
        <ul
          id={listId}
          className="mt-2 max-h-72 space-y-1 overflow-auto text-sm text-slate-600 dark:text-slate-300"
        >
          {ignored.map((item) => (
            <li key={item.id}>
              <span className="font-medium">{item.name}</span>
              <span className="ml-2 text-slate-500 dark:text-slate-400">— {item.reason}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
