import { useId, useState } from 'react';
import { Button } from '../design/Button';
import { isOutsideRun } from '../api/contract';
import type { IgnoreItem } from '../api/contract';

/**
 * The ignored tables and columns, each with its required reason.  The count is
 * always shown; the list itself opens on request.  Tables a run of chosen
 * tables left out are only counted here: their own report lists them.
 */
export function IgnoredItems({ ignored }: { ignored: IgnoreItem[] }) {
  const [open, setOpen] = useState(false);
  const listId = useId();
  // Tables a run of chosen tables left out are counted, not listed: their own
  // report lists them.  A person's own ignores are listed on request.
  const outside = ignored.filter((i) => isOutsideRun(i.reason)).length;
  const own = ignored.filter((i) => !isOutsideRun(i.reason));
  if (ignored.length === 0) {
    return null;
  }
  const count = own.length === 1 ? '1 item' : `${own.length} items`;
  return (
    <div className="mt-3" data-testid="ignored-items">
      <div className="flex flex-wrap items-center gap-3">
        <h3 className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">
          Ignored
        </h3>
        {own.length > 0 && (
          <span className="text-sm text-slate-700 dark:text-slate-300">
            {count}, each with its reason
          </span>
        )}
        {outside > 0 && (
          <span className="text-sm text-slate-700 dark:text-slate-300" data-testid="not-in-run">
            {outside === 1 ? '1 table was' : `${outside} tables were`} not part of this run (only the
            chosen tables were checked); the Report step can export the list.
          </span>
        )}
        {own.length > 0 && (
          <Button
            size="sm"
            variant="secondary"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            aria-controls={listId}
          >
            {open ? 'Hide ignored items' : 'Show ignored items'}
          </Button>
        )}
      </div>
      {open && (
        <ul
          id={listId}
          className="mt-2 max-h-72 space-y-1 overflow-auto text-sm text-slate-600 dark:text-slate-300"
        >
          {own.map((item) => (
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
