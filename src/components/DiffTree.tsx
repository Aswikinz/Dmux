import { ChevronRight, ChevronDown } from 'lucide-react';
import { useState } from 'react';
import { cn } from '../design/cn';
import { StatusBadge } from '../design/StatusBadge';
import type { SchemaDiffNode } from '../api/contract';

interface DiffTreeProps {
  nodes: SchemaDiffNode[];
  showOnlyProblems?: boolean;
  className?: string;
}

function statusColor(status: SchemaDiffNode['status']): string {
  switch (status) {
    case 'missing':
      return 'text-red-700 dark:text-red-300';
    case 'added':
      return 'text-emerald-700 dark:text-emerald-300';
    case 'changed':
      return 'text-amber-700 dark:text-amber-300';
    default:
      return 'text-slate-700 dark:text-slate-300';
  }
}

interface DiffNodeRowProps {
  node: SchemaDiffNode;
  depth: number;
  showOnlyProblems: boolean;
}

function DiffNodeRow({ node, depth, showOnlyProblems }: DiffNodeRowProps) {
  const [expanded, setExpanded] = useState(true);
  const hasChildren = (node.children?.length ?? 0) > 0;
  const isProblem = node.status !== 'same';

  if (showOnlyProblems && !isProblem && !hasChildren) {
    return null;
  }

  const filteredChildren = showOnlyProblems
    ? node.children?.filter((c) => c.status !== 'same' || (c.children?.some?.((cc) => cc.status !== 'same') ?? false))
    : node.children;

  return (
    <div role="treeitem" aria-expanded={hasChildren ? expanded : undefined}>
      <div
        className={cn(
          'flex items-center gap-2 py-1.5 pr-3 transition-colors hover:bg-black/[0.025] dark:hover:bg-white/[0.04]',
          
        )}
        style={{ paddingLeft: `${depth * 16 + 8}px` }}
      >
        {hasChildren ? (
          <button
            type="button"
            onClick={() => setExpanded((e) => !e)}
            className="rounded-md p-0.5 text-slate-500 dark:text-slate-400 transition-colors hover:bg-black/[0.06] dark:hover:bg-white/10"
            aria-label={expanded ? 'Collapse' : 'Expand'}
          >
            {expanded ? (
              <ChevronDown className="h-4 w-4" aria-hidden="true" />
            ) : (
              <ChevronRight className="h-4 w-4" aria-hidden="true" />
            )}
          </button>
        ) : (
          <span className="h-4 w-4" />
        )}
        <span className={cn('text-sm font-medium', statusColor(node.status))}>
          {node.name}
        </span>
        {node.status === 'changed' && (node.old || node.new) && (
          <span className="text-sm text-slate-700 dark:text-slate-300" data-testid="diff-before-after">
            <span className="sr-only">changed from </span>
            <code className="rounded bg-slate-100 px-1 dark:bg-slate-800">{node.old ?? '—'}</code>
            <span aria-hidden="true"> → </span>
            <span className="sr-only"> to </span>
            <code className="rounded bg-slate-100 px-1 dark:bg-slate-800">{node.new ?? '—'}</code>
          </span>
        )}
        {node.status !== 'same' && <StatusBadge status={node.status} className="ml-auto" />}
      </div>
      {expanded && filteredChildren && filteredChildren.length > 0 && (
        <div role="group">
          {filteredChildren.map((child) => (
            <DiffNodeRow
              key={child.id}
              node={child}
              depth={depth + 1}
              showOnlyProblems={showOnlyProblems}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export function DiffTree({ nodes, showOnlyProblems = false, className }: DiffTreeProps) {
  return (
    <div className={cn('overflow-auto rounded-xl border border-black/[0.06] py-1 dark:border-white/[0.08]', className)} role="tree" tabIndex={0}>
      {nodes.map((node) => (
        <DiffNodeRow
          key={node.id}
          node={node}
          depth={0}
          showOnlyProblems={showOnlyProblems}
        />
      ))}
    </div>
  );
}
