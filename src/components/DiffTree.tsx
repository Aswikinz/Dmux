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
          'flex items-center gap-2 py-1 pr-2 hover:bg-slate-50 dark:hover:bg-slate-800',
          depth > 0 && 'border-l border-slate-200 dark:border-slate-700',
        )}
        style={{ paddingLeft: `${depth * 16 + 8}px` }}
      >
        {hasChildren ? (
          <button
            type="button"
            onClick={() => setExpanded((e) => !e)}
            className="rounded p-0.5 hover:bg-slate-200 dark:hover:bg-slate-700"
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
    <div className={cn('overflow-auto rounded-lg border border-slate-200 dark:border-slate-700', className)} role="tree" tabIndex={0}>
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
