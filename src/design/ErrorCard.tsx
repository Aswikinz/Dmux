import { AlertTriangle } from 'lucide-react';
import { cn } from './cn';

interface ErrorCardProps {
  title: string;
  children: React.ReactNode;
  className?: string;
}

export function ErrorCard({ title, children, className }: ErrorCardProps) {
  return (
    <div
      className={cn(
        'animate-fade-up rounded-2xl border border-red-200 bg-red-50/80 p-4 text-red-900 dark:border-red-500/30 dark:bg-red-950/30 dark:text-red-100',
        className,
      )}
      role="alert"
    >
      <div className="flex items-start gap-3">
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-red-100 dark:bg-red-500/20">
          <AlertTriangle className="h-4 w-4 text-red-600 dark:text-red-300" aria-hidden="true" />
        </span>
        <div>
          <h3 className="text-sm font-semibold">{title}</h3>
          <div className="mt-0.5 text-sm text-red-800 dark:text-red-200">{children}</div>
        </div>
      </div>
    </div>
  );
}
