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
        'rounded-xl border-l-4 border-red-500 bg-red-50 p-4 text-red-900 dark:bg-red-900/20 dark:text-red-100',
        className,
      )}
      role="alert"
    >
      <div className="flex items-start gap-3">
        <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0" aria-hidden="true" />
        <div>
          <h3 className="font-semibold">{title}</h3>
          <div className="mt-1 text-sm">{children}</div>
        </div>
      </div>
    </div>
  );
}
