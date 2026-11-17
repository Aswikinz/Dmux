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
  current: Step;
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

export function StepSidebar({ steps, current, completed, onSelect }: StepSidebarProps) {
  return (
    <nav aria-label="Progress">
      <ol className="space-y-2">
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
                  'flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm font-medium transition-colors',
                  isCurrent
                    ? 'bg-blue-50 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300'
                    : 'text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800',
                )}
                aria-current={isCurrent ? 'step' : undefined}
              >
                <span
                  className={cn(
                    'flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs',
                    isDone
                      ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300'
                      : isCurrent
                        ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300'
                        : 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-400',
                  )}
                  aria-hidden="true"
                >
                  {isDone ? '✓' : index + 1}
                </span>
                {label}
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
