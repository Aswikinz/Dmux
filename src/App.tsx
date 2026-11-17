import { useCallback, useMemo, useState } from 'react';
import { ThemeProvider, useTheme } from './design/theme';
import appIcon from './assets/app-icon.png';
import { Button } from './design/Button';
import { StepSidebar, type Step } from './design/StepSidebar';
import { ConnectScreen, defaultOldProfile, defaultNewProfile } from './screens/ConnectScreen';
import { CompareStructureScreen } from './screens/CompareStructureScreen';
import { MapScreen } from './screens/MapScreen';
import { ChooseChecksScreen } from './screens/ChooseChecksScreen';
import { RunScreen } from './screens/RunScreen';
import { ResultsScreen } from './screens/ResultsScreen';
import { ReportScreen } from './screens/ReportScreen';
import { SettingsScreen } from './screens/SettingsScreen';
import { HistoryScreen } from './screens/HistoryScreen';
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
import { Moon, Sun, Zap, ZapOff, Menu, History, Settings } from 'lucide-react';

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
// is stubbed until T022), the deterministic mock in the browser / Vitest /
// Playwright, and a forced fixture for UI tests.

function AppShell() {
  const { resolvedMode, mode, setMode, reducedEffects, toggleReducedEffects } = useTheme();
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [engine] = useState<Engine>(() => resolveEngine());
  const [step, setStep] = useState<Step>('connect');
  // History and Settings are separate sections below the steps, not steps.
  const [view, setView] = useState<'steps' | 'history' | 'settings'>('steps');
  const [completed, setCompleted] = useState<Set<Step>>(new Set());

  const [oldProfile, setOldProfile] = useState<ConnectionProfile>(defaultOldProfile());
  const [newProfile, setNewProfile] = useState<ConnectionProfile>(defaultNewProfile());
  const [inventory, setInventory] = useState<Inventory | null>(null);
  const [mapping, setMapping] = useState<Mapping | null>(null);
  const [plan, setPlan] = useState<CheckPlan | null>(null);
  const [progress, setProgress] = useState<RunProgress | null>(null);
  const [results, setResults] = useState<Results | null>(null);
  // True while the Results screen shows results opened from History or a file.
  const [resultsOpened, setResultsOpened] = useState(false);

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
      if (view === 'history' || nextIndex <= currentIndex + 1) {
        setStep(next);
      }
      setView('steps');
    },
    [step, view],
  );

  // Verified results opened from the History screen (or a results file) are
  // shown on the Results screen; the Report screen can export them too.
  const showOpenedResults = useCallback((opened: Results) => {
    setResults(opened);
    setResultsOpened(true);
    setView('steps');
    setStep('results');
  }, []);

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
    setView('steps');
    setStep('connect');
    setCompleted(new Set());
    setInventory(null);
    setMapping(null);
    setPlan(null);
    setProgress(null);
    setResults(null);
    setResultsOpened(false);
    setOldProfile(defaultOldProfile());
    setNewProfile(defaultNewProfile());
  }, []);

  return (
    <div
      className={`flex min-h-screen text-ink dark:text-slate-100 ${
        reducedEffects
          ? 'bg-canvas dark:bg-canvas-dark'
          : 'bg-[radial-gradient(1200px_600px_at_100%_-10%,rgba(99,87,251,0.10),transparent_60%),radial-gradient(900px_500px_at_-10%_110%,rgba(16,207,174,0.07),transparent_60%)] bg-canvas dark:bg-canvas-dark dark:bg-[radial-gradient(1200px_600px_at_100%_-10%,rgba(99,87,251,0.18),transparent_60%)]'
      }`}
    >
      <aside
        className={`sticky top-0 h-screen shrink-0 flex-col border-r border-black/[0.06] px-3 py-4 dark:border-white/[0.08] ${
          reducedEffects ? 'bg-[#ececef] dark:bg-[#1e1e21]' : 'bg-white/60 backdrop-blur-2xl dark:bg-white/[0.04]'
        } ${sidebarOpen ? 'flex w-60' : 'hidden'}`}
      >
        <div className="mb-5 flex items-center gap-2.5 px-2.5 pt-1">
          <img
            src={appIcon}
            alt=""
            width={32}
            height={32}
            className="h-8 w-8 rounded-[8px] shadow-[0_2px_6px_rgba(42,36,135,0.35)]"
          />
          <div className="leading-tight">
            <div className="text-[15px] font-semibold tracking-[-0.01em]">Dmux</div>
            <div className="text-[11px] text-ink-secondary dark:text-slate-400">Migration verifier</div>
          </div>
        </div>
        <div className="mb-1 px-2.5 text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">
          Steps
        </div>
        <StepSidebar
          steps={stepLabels}
          current={view === 'steps' ? step : null}
          completed={completed}
          onSelect={goToStep}
        />
        <div className="mb-1 mt-5 px-2.5 text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-secondary dark:text-slate-400">
          Library
        </div>
        <nav aria-label="Library" className="space-y-0.5">
          {(
            [
              ['history', 'History', History],
              ['settings', 'Settings', Settings],
            ] as const
          ).map(([id, label, Icon]) => (
            <button
              key={id}
              type="button"
              onClick={() => setView(id)}
              aria-current={view === id ? 'page' : undefined}
              className={`flex h-9 w-full items-center gap-3 rounded-lg px-2.5 text-left text-[13px] font-medium transition-colors duration-150 ${
                view === id
                  ? 'bg-accent-600 text-white shadow-[0_1px_2px_rgba(30,27,75,0.3)] dark:bg-accent-500'
                  : 'text-slate-700 hover:bg-black/[0.05] dark:text-slate-300 dark:hover:bg-white/[0.07]'
              }`}
            >
              <Icon className="h-5 w-5 shrink-0" aria-hidden="true" />
              <span className="truncate">{label}</span>
            </button>
          ))}
        </nav>
        <div className="mt-auto flex items-center gap-1 px-1">
          <Button
            variant="ghost"
            size="sm"
            className="h-8 w-8 px-0 text-ink-secondary dark:text-slate-400"
            onClick={() => setMode(resolvedMode === 'dark' ? 'light' : 'dark')}
            aria-label={resolvedMode === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
            title={resolvedMode === 'dark' ? 'Light appearance' : 'Dark appearance'}
          >
            {resolvedMode === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="h-8 w-8 px-0 text-ink-secondary dark:text-slate-400"
            onClick={toggleReducedEffects}
            aria-label={reducedEffects ? 'Disable reduced effects' : 'Enable reduced effects'}
            title={reducedEffects ? 'Reduced effects are on' : 'Reduce visual effects'}
          >
            {reducedEffects ? <ZapOff className="h-4 w-4" /> : <Zap className="h-4 w-4" />}
          </Button>
          <span className="sr-only">
            Appearance: {mode}. Effects: {reducedEffects ? 'reduced' : 'full'}.
          </span>
        </div>
      </aside>
      <main className="flex min-w-0 flex-1 flex-col px-8 py-7">
        <div className="mb-4 flex items-center gap-3 lg:hidden">
          <Button variant="ghost" size="sm" onClick={() => setSidebarOpen((s) => !s)} aria-label="Toggle sidebar">
            <Menu className="h-5 w-5" />
          </Button>
        </div>
        {view === 'settings' && (
          <div key="settings" className="mx-auto w-full max-w-6xl animate-fade-up">
            <SettingsScreen engine={engine} />
          </div>
        )}
        <div
          key={view === 'history' ? 'history' : step}
          hidden={view === 'settings'}
          className="mx-auto w-full max-w-6xl animate-fade-up"
        >
          {view === 'history' && (
            <HistoryScreen engine={engine} onOpenResults={showOpenedResults} />
          )}
          {view !== 'history' && step === 'connect' && (
            <ConnectScreen
              engine={engine}
              oldProfile={oldProfile}
              newProfile={newProfile}
              onChangeOld={setOldProfile}
              onChangeNew={setNewProfile}
              onNext={advance}
            />
          )}
          {view !== 'history' && step === 'structure' && (
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
          {view !== 'history' && step === 'map' && inventory && (
            <MapScreen
              engine={engine}
              inventory={inventory}
              mapping={mapping}
              onMappingChange={(m) => {
                setMapping(m);
                // A different mapping means a different set of tables to check.
                setPlan(null);
                markCompleted('map');
              }}
              onNext={advance}
              onBack={back}
            />
          )}
          {view !== 'history' && step === 'checks' && mapping && (
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
          {view !== 'history' && step === 'run' && definition && (
            <RunScreen
              engine={engine}
              definition={definition}
              progress={progress}
              onProgressChange={(p) => {
                // A different run replaces any results opened from History.
                if (p.runId !== progress?.runId) {
                  setResults(null);
                  setResultsOpened(false);
                }
                setProgress(p);
                if (p.overallState === 'done' || p.overallState === 'cancelled') {
                  markCompleted('run');
                }
              }}
              onNext={advance}
              onBack={back}
            />
          )}
          {view !== 'history' && step === 'results' && (progress || results) && (
            <ResultsScreen
              engine={engine}
              runId={results?.runId ?? progress?.runId ?? ''}
              results={results}
              onResultsChange={(r) => {
                setResults(r);
                markCompleted('results');
              }}
              onNext={advance}
              onBack={resultsOpened ? () => setView('history') : back}
            />
          )}
          {view !== 'history' && step === 'report' && results && (
            <ReportScreen engine={engine} results={results} onBack={back} onRestart={restart} />
          )}
        </div>
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
