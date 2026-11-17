import { useEffect, useRef, useState } from 'react';
import { Ban, Check, Undo2 } from 'lucide-react';
import { cn } from '../design/cn';
import { Button } from '../design/Button';
import { StatusBadge } from '../design/StatusBadge';
import type { MappingMatch } from '../api/contract';

interface MappingQueueProps {
  queue: MappingMatch[];
  onApprove: (id: string) => void;
  onIgnore: (id: string, reason: string) => void;
  className?: string;
}

export function MappingQueue({ queue, onApprove, onIgnore, className }: MappingQueueProps) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [ignoreReason, setIgnoreReason] = useState('');
  const [ignoringId, setIgnoringId] = useState<string | null>(null);
  // Whether the ignore prompt was opened with the keyboard (Delete/Backspace),
  // so we only move focus for keyboard users, not mouse users (a11y rule:
  // no autoFocus that yanks focus away from a pointer interaction).
  const [ignoringViaKeyboard, setIgnoringViaKeyboard] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const ignoreInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setSelectedIndex((prev) => Math.min(prev, Math.max(0, queue.length - 1)));
  }, [queue.length]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (queue.length === 0) return;

      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, queue.length - 1));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === 'Enter') {
        e.preventDefault();
        if (ignoringId) {
          if (ignoreReason.trim()) {
            onIgnore(ignoringId, ignoreReason.trim());
            setIgnoringId(null);
            setIgnoreReason('');
          }
        } else {
          onApprove(queue[selectedIndex]?.id ?? '');
        }
      } else if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        if (!ignoringId) {
          setIgnoringViaKeyboard(true);
          setIgnoringId(queue[selectedIndex]?.id ?? null);
        }
      } else if (e.key === 'Escape') {
        setIgnoringId(null);
        setIgnoreReason('');
      }
    }

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [queue, selectedIndex, ignoringId, ignoreReason, onApprove, onIgnore]);

  useEffect(() => {
    const el = listRef.current?.querySelector(`[data-index="${selectedIndex}"]`);
    if (el) {
      el.scrollIntoView({ block: 'nearest' });
    }
  }, [selectedIndex]);

  // Move focus to the reason input only when the ignore prompt was opened with
  // the keyboard, so keyboard navigation stays complete without stealing focus
  // from a mouse click (replaces autoFocus, a11y rule S9379).
  useEffect(() => {
    if (ignoringViaKeyboard && ignoringId) {
      ignoreInputRef.current?.focus();
      setIgnoringViaKeyboard(false);
    }
  }, [ignoringViaKeyboard, ignoringId]);

  if (queue.length === 0) {
    return (
      <div className={cn('text-sm text-slate-600 dark:text-slate-300', className)}>
        No items need attention.
      </div>
    );
  }

  return (
    <div className={cn('flex flex-col gap-3', className)}>
      <div className="flex items-center justify-between">
        <span className="text-sm text-slate-600 dark:text-slate-300">
          {queue.length} item{queue.length === 1 ? '' : 's'} need attention
        </span>
        <span className="text-xs text-slate-500">Use ↑ ↓ Enter to approve, Delete to ignore</span>
      </div>
      <div
        ref={listRef}
        className="max-h-[50vh] overflow-auto rounded-lg border border-slate-200 dark:border-slate-700"
        tabIndex={0}
        role="listbox"
        aria-label="Mapping queue"
      >
        {queue.map((item, index) => {
          const isSelected = index === selectedIndex;
          const isIgnoring = ignoringId === item.id;
          return (
            <div
              key={item.id}
              data-index={index}
              role="option"
              aria-selected={isSelected}
              className={cn(
                'border-b border-slate-100 p-3 last:border-0 dark:border-slate-800',
                isSelected
                  ? 'bg-blue-50 dark:bg-blue-900/20'
                  : 'bg-white dark:bg-slate-900',
              )}
            >
              <div className="flex items-center gap-3">
                <span className="text-sm font-medium text-slate-800 dark:text-slate-100">
                  {item.oldName}
                </span>
                <span className="text-slate-400">→</span>
                <span className="text-sm font-medium text-slate-800 dark:text-slate-100">
                  {item.newName}
                </span>
                <StatusBadge status={item.confidence === 'exact' ? 'same' : 'changed'} className="ml-auto" />
              </div>
              {isSelected && (
                <div className="mt-3 flex flex-wrap items-center gap-2">
                  {isIgnoring ? (
                    <>
                      <input
                        ref={ignoreInputRef}
                        type="text"
                        value={ignoreReason}
                        onChange={(e) => setIgnoreReason(e.target.value)}
                        placeholder="Reason for ignoring (required)"
                        aria-label="Reason for ignoring"
                        className="flex-1 rounded-lg border border-slate-300 px-3 py-1.5 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100"
                      />
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => {
                          setIgnoringId(null);
                          setIgnoreReason('');
                        }}
                      >
                        <Undo2 className="h-4 w-4" />
                        Cancel
                      </Button>
                      <Button
                        size="sm"
                        variant="danger"
                        disabled={!ignoreReason.trim()}
                        onClick={() => {
                          onIgnore(item.id, ignoreReason.trim());
                          setIgnoringId(null);
                          setIgnoreReason('');
                        }}
                      >
                        <Ban className="h-4 w-4" />
                        Ignore
                      </Button>
                    </>
                  ) : (
                    <>
                      <Button size="sm" variant="primary" onClick={() => onApprove(item.id)}>
                        <Check className="h-4 w-4" />
                        Approve
                      </Button>
                      <Button size="sm" variant="secondary" onClick={() => setIgnoringId(item.id)}>
                        <Ban className="h-4 w-4" />
                        Ignore
                      </Button>
                    </>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
