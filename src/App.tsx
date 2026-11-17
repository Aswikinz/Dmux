import { useCallback, useMemo, useState } from 'react';
import { ThemeProvider, useTheme } from './design/theme';
import { Panel } from './design/Panel';
import { Button } from './design/Button';
import { StepSidebar, type Step } from './design/StepSidebar';
import { ConnectScreen, defaultOldProfile, defaultNewProfile } from './screens/ConnectScreen';
import { CompareStructureScreen } from './screens/CompareStructureScreen';
import { MapScreen } from './screens/MapScreen';
import { ChooseChecksScreen } from './screens/ChooseChecksScreen';
import { RunScreen } from './screens/RunScreen';
import { ResultsScreen } from './screens/ResultsScreen';
import { ReportScreen } from './screens/ReportScreen';
import type { Engine } from './api/engine';
import { resolveEngine } from './api/engineSelection';
import type {
  ConnectionProfile,
  Inventory,
  Mapping,
  CheckPlan,
  RunDefinition,
  RunProgress,
  Results,
} from './api/contract';
import { Moon, Sun, Zap, ZapOff, Menu } from 'lucide-react';

const stepOrder: Step[] = [
  'connect',
  'structure',
  'map',
  'checks',
  'run',
  'results',
  'report',
];

const stepLabels: { id: Step; label: string }[] = [
  { id: 'connect', label: 'Connect' },
  { id: 'structure', label: 'Compare structure' },
  { id: 'map', label: 'Map' },
  { id: 'checks', label: 'Choose checks' },
  { id: 'run', label: 'Run' },
  { id: 'results', label: 'Results' },
  { id: 'report', label: 'Report' },
];

// The engine is resolved per runtime (see src/api/engineSelection.ts): the real
// Tauri backend inside the desktop app for the structure half (the data half
// still mocks until T022), the deterministic mock in the browser / Vitest /
// Playwright, and a forced fixture for UI tests.

function AppShell() {
  const { resolvedMode, mode, setMode, reducedEffects, toggleReducedEffects } = useTheme();
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [engine] = useState<Engine>(() => resolveEngine());
  const [step, setStep] = useState<Step>('connect');
  const [completed, setCompleted] = useState<Set<Step>>(new Set());

  const [oldProfile, setOldProfile] = useState<ConnectionProfile>(defaultOldProfile());
  const [newProfile, setNewProfile] = useState<ConnectionProfile>(defaultNewProfile());
  const [inventory, setInventory] = useState<Inventory | null>(null);
  const [mapping, setMapping] = useState<Mapping | null>(null);
  const [plan, setPlan] = useState<CheckPlan | null>(null);
  const [progress, setProgress] = useState<RunProgress | null>(null);
  const [results, setResults] = useState<Results | null>(null);

  const definition = useMemo<RunDefinition | null>(
    () =>
      plan
        ? { oldProfile, newProfile, mapping: mapping ?? { rules: [], matches: [], needsAttention: [], ignored: [] }, plan }
        : null,
    [plan, mapping, oldProfile, newProfile],
  );

  const goToStep = useCallback(
    (next: Step) => {
      const currentIndex = stepOrder.indexOf(step);
      const nextIndex = stepOrder.indexOf(next);
      if (nextIndex <= currentIndex + 1) {
        setStep(next);
      }
    },
    [step],
  );

  const markCompleted = useCallback((s: Step) => {
    setCompleted((prev) => new Set([...prev, s]));
  }, []);

  const advance = useCallback(() => {
    markCompleted(step);
    const index = stepOrder.indexOf(step);
    if (index < stepOrder.length - 1) {
      setStep(stepOrder[index + 1]);
    }
  }, [step, markCompleted]);

  const back = useCallback(() => {
    const index = stepOrder.indexOf(step);
    if (index > 0) {
      setStep(stepOrder[index - 1]);
    }
  }, [step]);

  const restart = useCallback(() => {
    setStep('connect');
    setCompleted(new Set());
    setInventory(null);
    setMapping(null);
    setPlan(null);
    setProgress(null);
    setResults(null);
    setOldProfile(defaultOldProfile());
    setNewProfile(defaultNewProfile());
  }, []);

  return (
    <div
      className={`flex min-h-screen ${
        reducedEffects
          ? 'bg-slate-50 text-slate-900 dark:bg-slate-950 dark:text-slate-100'
          : 'bg-gradient-to-br from-blue-50 to-indigo-100 text-slate-900 dark:from-slate-950 dark:to-slate-900 dark:text-slate-100'
      }`}
    >
      <aside
        className={`shrink-0 flex-col gap-4 border-r border-white/30 bg-white/75 p-4 backdrop-blur-[16px] transition-all dark:border-white/10 dark:bg-slate-900/75 ${
          sidebarOpen ? 'flex w-64' : 'hidden'
        }`}
      >
        <div className="text-xl font-bold">Dmux</div>
        <StepSidebar
          steps={stepLabels}
          current={step}
          completed={completed}
          onSelect={goToStep}
        />
        <div className="mt-auto space-y-2">
          <div className="flex gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setMode(resolvedMode === 'dark' ? 'light' : 'dark')}
              aria-label={resolvedMode === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
            >
              {resolvedMode === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={toggleReducedEffects}
              aria-label={reducedEffects ? 'Disable reduced effects' : 'Enable reduced effects'}
            >
              {reducedEffects ? <ZapOff className="h-4 w-4" /> : <Zap className="h-4 w-4" />}
            </Button>
          </div>
          <div className="text-xs text-slate-500 dark:text-slate-400">
            Mode: {mode} • Effects: {reducedEffects ? 'reduced' : 'glass'}
          </div>
        </div>
      </aside>
      <main className="flex flex-1 flex-col p-6">
        <div className="mb-4 flex items-center gap-3 lg:hidden">
          <Button variant="ghost" size="sm" onClick={() => setSidebarOpen((s) => !s)} aria-label="Toggle sidebar">
            <Menu className="h-5 w-5" />
          </Button>
        </div>
        <Panel className="flex-1">
          {step === 'connect' && (
            <ConnectScreen
              engine={engine}
              oldProfile={oldProfile}
              newProfile={newProfile}
              onChangeOld={setOldProfile}
              onChangeNew={setNewProfile}
              onNext={advance}
            />
          )}
          {step === 'structure' && (
            <CompareStructureScreen
              engine={engine}
              oldProfile={oldProfile}
              newProfile={newProfile}
              inventory={inventory}
              onInventoryChange={(inv) => {
                setInventory(inv);
                markCompleted('structure');
              }}
              onNext={advance}
              onBack={back}
            />
          )}
          {step === 'map' && inventory && (
            <MapScreen
              engine={engine}
              inventory={inventory}
              mapping={mapping}
              onMappingChange={(m) => {
                setMapping(m);
                markCompleted('map');
              }}
              onNext={advance}
              onBack={back}
            />
          )}
          {step === 'checks' && mapping && (
            <ChooseChecksScreen
              engine={engine}
              mapping={mapping}
              plan={plan}
              onPlanChange={(p) => {
                setPlan(p);
                markCompleted('checks');
              }}
              onNext={advance}
              onBack={back}
            />
          )}
          {step === 'run' && definition && (
            <RunScreen
              engine={engine}
              definition={definition}
              progress={progress}
              onProgressChange={(p) => {
                setProgress(p);
                if (p.overallState === 'done' || p.overallState === 'cancelled') {
                  markCompleted('run');
                }
              }}
              onNext={advance}
              onBack={back}
            />
          )}
          {step === 'results' && progress && (
            <ResultsScreen
              engine={engine}
              runId={progress.runId}
              results={results}
              onResultsChange={(r) => {
                setResults(r);
                markCompleted('results');
              }}
              onNext={advance}
              onBack={back}
            />
          )}
          {step === 'report' && results && (
            <ReportScreen engine={engine} results={results} onBack={back} onRestart={restart} />
          )}
        </Panel>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <ThemeProvider>
      <AppShell />
    </ThemeProvider>
  );
}
