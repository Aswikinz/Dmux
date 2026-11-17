import { ShieldAlert } from 'lucide-react';
import { cn } from '../design/cn';

/** The honest custom-check warning.  The wording is fixed on purpose: scripts
 *  are not sandboxed, and the user must not be able to miss that. */
export const HOOK_WARNING_TEXT =
  'A custom check runs with your full permissions. Only run scripts you trust.';

interface HookWarningProps {
  className?: string;
  /** Extra plain-words line under the fixed warning. */
  children?: React.ReactNode;
}

/** A prominent amber notice; the icon is decoration, the words carry the meaning. */
export function HookWarning({ className, children }: HookWarningProps) {
  return (
    <div
      role="note"
      data-testid="hook-warning"
      className={cn(
        'flex items-start gap-3 rounded-xl border border-amber-300 bg-amber-50 p-4 text-amber-950 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-50',
        className,
      )}
    >
      <ShieldAlert className="mt-0.5 h-5 w-5 shrink-0 text-amber-700 dark:text-amber-300" aria-hidden="true" />
      <div className="text-sm">
        <p className="font-semibold">{HOOK_WARNING_TEXT}</p>
        {children && <div className="mt-1 text-amber-900 dark:text-amber-100">{children}</div>}
      </div>
    </div>
  );
}
