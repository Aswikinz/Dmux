import { Check } from 'lucide-react';
import { cn } from './cn';

export type Step =
  | 'connect'
  | 'structure'
  | 'map'
  | 'checks'
  | 'run'
  | 'results'
  | 'report';

interface StepSidebarProps {
  steps: { id: Step; label: string }[];
  /** The current step, or null while another section (such as History) is shown. */
  current: Step | null;
  completed: Set<Step>;
  onSelect: (step: Step) => void;
}

const allSteps: Step[] = [
  'connect',
  'structure',
  'map',
  'checks',
  'run',
  'results',
  'report',
];

/** The step list, styled like a macOS source list. */
export function StepSidebar({ steps, current, completed, onSelect }: StepSidebarProps) {
  return (
    <nav aria-label="Progress">
      <ol className="space-y-0.5">
        {steps.map(({ id, label }) => {
          const index = allSteps.indexOf(id);
          const isCurrent = id === current;
          const isDone = completed.has(id);
          return (
            <li key={id}>
              <button
                type="button"
                onClick={() => onSelect(id)}
                className={cn(
                  'group flex h-9 w-full items-center gap-3 rounded-lg px-2.5 text-left text-[13px] font-medium transition-colors duration-150',
                  isCurrent
                    ? 'bg-accent-600 text-white shadow-[0_1px_2px_rgba(30,27,75,0.3)] dark:bg-accent-500'
                    : 'text-slate-700 hover:bg-black/[0.05] dark:text-slate-300 dark:hover:bg-white/[0.07]',
                )}
                aria-current={isCurrent ? 'step' : undefined}
              >
                <span
                  className={cn(
                    'flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold tabular-nums transition-colors',
                    isCurrent
                      ? 'bg-white/25 text-white'
                      : isDone
                        ? 'animate-pop bg-mint-500 text-white'
                        : 'bg-black/[0.07] text-slate-600 dark:bg-white/10 dark:text-slate-300',
                  )}
                  aria-hidden="true"
                >
                  {isDone && !isCurrent ? <Check className="h-3 w-3" strokeWidth={3.5} /> : index + 1}
                </span>
                <span className="truncate">{label}</span>
                {isDone && <span className="sr-only">(done)</span>}
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
