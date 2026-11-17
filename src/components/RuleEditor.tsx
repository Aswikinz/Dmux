import { useState } from 'react';
import { describeError } from '../api/tauriEngine';
import { Plus, Trash2 } from 'lucide-react';
import { Button } from '../design/Button';
import { Input } from '../design/Input';
import { Select } from '../design/Select';
import type { MappingRule, RulePreview } from '../api/contract';

interface RuleEditorProps {
  /** The rules the current mapping was built with. */
  rules: MappingRule[];
  /** Count how many tables these rules pair.  Changes nothing. */
  onPreview: (rules: MappingRule[]) => Promise<RulePreview>;
  /** Build the mapping again with these rules. */
  onApply: (rules: MappingRule[]) => Promise<void>;
}

type RuleKind = MappingRule['kind'];

const KIND_OPTIONS: { value: RuleKind; label: string }[] = [
  { value: 'prefix_replace', label: 'Table name starts with' },
  { value: 'suffix_replace', label: 'Table name ends with' },
];

function ruleIsComplete(rule: MappingRule): boolean {
  if (rule.kind === 'schema_rename') return rule.pattern.trim() !== '' && rule.target.trim() !== '';
  return rule.pattern !== '' || rule.target !== '';
}

/**
 * Rename rules, applied before names are matched.  A rule renames a schema or
 * the start or end of a table name; it never renames a column.  Changing the
 * rules does nothing until the user previews them and then applies them.
 */
export function RuleEditor({ rules, onPreview, onApply }: RuleEditorProps) {
  const [draft, setDraft] = useState<MappingRule[]>(rules);
  const [preview, setPreview] = useState<RulePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [applied, setApplied] = useState(false);

  const complete = draft.every(ruleIsComplete);

  function change(next: MappingRule[]) {
    setDraft(next);
    setPreview(null);
    setApplied(false);
    setError(null);
  }

  function update(index: number, patch: Partial<MappingRule>) {
    change(draft.map((r, i) => (i === index ? { ...r, ...patch } : r)));
  }

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3" data-testid="rule-editor">
      <p className="text-sm text-slate-600 dark:text-slate-300">
        Rules come first. They rename schemas and the start or end of table names, so the old and
        new tables can be paired. They never rename columns.
      </p>
      {draft.length === 0 && (
        <p className="text-sm text-slate-600 dark:text-slate-300">No rules yet.</p>
      )}
      <ul className="space-y-2">
        {draft.map((rule, index) => {
          const isSchema = rule.kind === 'schema_rename';
          const n = index + 1;
          return (
            <li key={index} className="flex flex-wrap items-end gap-2">
              {isSchema ? (
                <span className="pb-2 text-sm font-medium text-slate-700 dark:text-slate-200">
                  Schema
                </span>
              ) : (
                <Select
                  label={`Rule ${n} type`}
                  options={KIND_OPTIONS}
                  value={rule.kind}
                  onChange={(e) => update(index, { kind: e.target.value as RuleKind })}
                  className="w-52"
                />
              )}
              <Input
                label={isSchema ? `Rule ${n}: old schema` : `Rule ${n}: old text`}
                value={rule.pattern}
                onChange={(e) => update(index, { pattern: e.target.value })}
                className="w-44"
              />
              <span className="pb-2 text-slate-600 dark:text-slate-400" aria-hidden="true">
                becomes
              </span>
              <Input
                label={isSchema ? `Rule ${n}: new schema` : `Rule ${n}: new text`}
                value={rule.target}
                onChange={(e) => update(index, { target: e.target.value })}
                className="w-44"
              />
              <Button
                size="sm"
                variant="ghost"
                aria-label={`Remove rule ${n}`}
                onClick={() => change(draft.filter((_, i) => i !== index))}
              >
                <Trash2 className="h-4 w-4" aria-hidden="true" />
                Remove
              </Button>
            </li>
          );
        })}
      </ul>
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="secondary"
          onClick={() => change([...draft, { kind: 'schema_rename', pattern: '', target: '' }])}
        >
          <Plus className="h-4 w-4" aria-hidden="true" />
          Add schema rename
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => change([...draft, { kind: 'prefix_replace', pattern: '', target: '' }])}
        >
          <Plus className="h-4 w-4" aria-hidden="true" />
          Add prefix or suffix rule
        </Button>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          disabled={busy || !complete}
          onClick={() => run(async () => setPreview(await onPreview(draft)))}
        >
          Preview rules
        </Button>
        <Button
          size="sm"
          variant="primary"
          disabled={busy || !complete}
          onClick={() =>
            run(async () => {
              await onApply(draft);
              setApplied(true);
            })
          }
        >
          Apply rules and rebuild mapping
        </Button>
      </div>
      {preview && (
        <p role="status" className="text-sm text-slate-700 dark:text-slate-300">
          These rules pair <strong>{preview.tablesByRules}</strong> of {preview.tablesTotal} tables.
          With names that already match, {preview.tablesMatched} of {preview.tablesTotal} tables
          would be paired.
        </p>
      )}
      {applied && (
        <p role="status" className="text-sm text-slate-700 dark:text-slate-300">
          The mapping was rebuilt with these rules.
        </p>
      )}
      <p className="text-xs text-slate-600 dark:text-slate-400">
        Applying rules rebuilds the mapping and clears any pairing or ignore you set by hand.
      </p>
      {error && (
        <p role="alert" className="text-sm text-red-700 dark:text-red-300">
          {error}
        </p>
      )}
    </div>
  );
}
