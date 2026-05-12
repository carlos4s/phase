import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import { useNavigate, useSearchParams } from "react-router";

import { useDraftStore } from "../stores/draftStore";
import { CardPreview } from "../components/card/CardPreview";
import type { CardHoverInfo } from "../components/card/CardPreview";
import { DraftIntro } from "../components/draft/DraftIntro";
import { DraftSteps } from "../components/draft/DraftSteps";
import { SetSelector } from "../components/draft/SetSelector";
import { PackDisplay } from "../components/draft/PackDisplay";
import { PoolPanel } from "../components/draft/PoolPanel";
import { DraftProgress } from "../components/draft/DraftProgress";
import { LimitedDeckBuilder } from "../components/draft/LimitedDeckBuilder";
import { ScreenChrome } from "../components/chrome/ScreenChrome";
import { menuButtonClass } from "../components/menu/buttonStyles";
import { runLimits } from "../services/quickDraftPersistence";
import type { DraftRunFormat, DraftRunState } from "../services/quickDraftPersistence";

// ── Format Picker ─────────────────────────────────────────────────────

const FORMAT_OPTIONS: Array<{ value: DraftRunFormat; label: string; description: string }> = [
  { value: "single", label: "Single Match", description: "Play one Bo1 match with your drafted deck." },
  { value: "bo3", label: "Best of Three", description: "Play a Bo3 match with sideboarding between games." },
  { value: "run", label: "Full Run", description: "Play Bo1 matches until you reach 7 wins or 3 losses." },
];

function FormatPicker({ onLaunch }: { onLaunch: () => void }) {
  const runFormat = useDraftStore((s) => s.runFormat);
  const setRunFormat = useDraftStore((s) => s.setRunFormat);

  return (
    <div className="flex flex-col items-center gap-8 py-16">
      <div className="text-center">
        <h1 className="menu-display text-3xl text-white">Your deck is ready</h1>
        <p className="mt-2 text-sm text-white/45">Choose how you want to play.</p>
      </div>

      <div className="flex w-full max-w-lg flex-col gap-3">
        {FORMAT_OPTIONS.map((opt) => (
          <button
            key={opt.value}
            type="button"
            onClick={() => setRunFormat(opt.value)}
            className={`group flex w-full cursor-pointer items-start gap-4 rounded-[18px] border p-4 text-left transition-colors ${
              runFormat === opt.value
                ? "border-emerald-400/30 bg-emerald-500/[0.08]"
                : "border-white/10 bg-white/[0.02] hover:border-white/18 hover:bg-white/[0.05]"
            }`}
          >
            <div
              className={`mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border-2 transition-colors ${
                runFormat === opt.value
                  ? "border-emerald-400 bg-emerald-400"
                  : "border-white/25"
              }`}
            >
              {runFormat === opt.value && (
                <div className="h-2 w-2 rounded-full bg-gray-950" />
              )}
            </div>
            <div className="min-w-0 flex-1">
              <div className={`text-base font-semibold ${runFormat === opt.value ? "text-emerald-100" : "text-white"}`}>
                {opt.label}
              </div>
              <p className="mt-1 text-sm text-white/40">{opt.description}</p>
            </div>
          </button>
        ))}
      </div>

      <button
        type="button"
        onClick={onLaunch}
        className={menuButtonClass({ tone: "emerald", size: "lg" })}
      >
        Start Match
      </button>
    </div>
  );
}

// ── Between Matches ───────────────────────────────────────────────────

function BetweenMatches({ onNext, onEnd }: { onNext: () => void; onEnd: () => void }) {
  const runState = useDraftStore((s) => s.runState);
  const runFormat = useDraftStore((s) => s.runFormat);

  if (!runState) return null;

  const { wins, losses, draws } = tallyResults(runState.results);
  const limits = runLimits(runFormat);
  const matchNumber = runState.results.length + 1;

  return (
    <div className="flex flex-col items-center gap-8 py-16">
      <h1 className="menu-display text-3xl text-white">Draft Run</h1>

      <RecordSummary wins={wins} losses={losses} draws={draws} limits={limits} />

      <MatchHistory results={runState.results} />

      <p className="text-sm text-white/45">Up next — Match {matchNumber}</p>

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={onNext}
          className={menuButtonClass({ tone: "emerald", size: "lg" })}
        >
          Next Match
        </button>
        <button
          type="button"
          onClick={onEnd}
          className={menuButtonClass({ tone: "neutral", size: "md" })}
        >
          End Run
        </button>
      </div>
    </div>
  );
}

// ── Run Complete ──────────────────────────────────────────────────────

function RunComplete({ onDone }: { onDone: () => void }) {
  const runState = useDraftStore((s) => s.runState);
  const runFormat = useDraftStore((s) => s.runFormat);

  if (!runState) return null;

  const { wins, losses, draws } = tallyResults(runState.results);
  const limits = runLimits(runFormat);
  const hitMaxWins = wins >= limits.maxWins;
  const perfect = hitMaxWins && losses === 0;

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4, ease: "easeOut" }}
      className="flex flex-col items-center gap-8 py-16"
    >
      <div className="relative flex flex-col items-center gap-2">
        {hitMaxWins && (
          <motion.div
            aria-hidden="true"
            className="pointer-events-none absolute -inset-x-20 -inset-y-10 rounded-full bg-emerald-400/15 blur-3xl"
            initial={{ opacity: 0, scale: 0.6 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ delay: 0.15, duration: 0.6, ease: "easeOut" }}
          />
        )}
        <h1 className="menu-display relative text-3xl text-white">
          {perfect ? "Perfect Run" : hitMaxWins ? "Run Complete" : "Run Over"}
        </h1>
        <p className="relative text-white/55">
          {hitMaxWins
            ? `You finished ${wins}–${losses}. ${perfect ? "Flawless." : "Congratulations!"}`
            : `You finished with a ${wins}–${losses} record.`}
        </p>
      </div>

      <RecordSummary wins={wins} losses={losses} draws={draws} limits={limits} />

      <MatchHistory results={runState.results} />

      <button
        type="button"
        onClick={onDone}
        className={menuButtonClass({ tone: "neutral", size: "lg" })}
      >
        Done
      </button>
    </motion.div>
  );
}

// ── Shared sub-components ─────────────────────────────────────────────

function tallyResults(results: DraftRunState["results"]): { wins: number; losses: number; draws: number } {
  let wins = 0;
  let losses = 0;
  let draws = 0;
  for (const r of results) {
    if (r.result === "win") wins += 1;
    else if (r.result === "loss") losses += 1;
    else draws += 1;
  }
  return { wins, losses, draws };
}

function RecordSummary({
  wins,
  losses,
  draws,
  limits,
}: {
  wins: number;
  losses: number;
  draws: number;
  limits: { maxWins: number; maxLosses: number };
}) {
  return (
    <div className="flex flex-col items-center gap-2">
      <div className="flex items-center gap-8">
        <RecordTrack label="Wins" count={wins} max={limits.maxWins} color="emerald" />
        <RecordTrack label="Losses" count={losses} max={limits.maxLosses} color="red" />
      </div>
      {draws > 0 && (
        <span className="text-xs uppercase tracking-wider text-white/35">
          + {draws} draw{draws > 1 ? "s" : ""}
        </span>
      )}
    </div>
  );
}

function RecordTrack({
  label,
  count,
  max,
  color,
}: {
  label: string;
  count: number;
  max: number;
  color: "emerald" | "red";
}) {
  const palette = {
    emerald: { filled: "border-emerald-300 bg-emerald-400 shadow-[0_0_8px] shadow-emerald-400/50", empty: "border-emerald-400/25", text: "text-emerald-200" },
    red: { filled: "border-red-300 bg-red-400 shadow-[0_0_8px] shadow-red-400/50", empty: "border-red-400/25", text: "text-red-200" },
  }[color];
  return (
    <div className="flex flex-col items-center gap-2">
      <div className="flex items-center gap-1.5">
        {Array.from({ length: max }, (_, i) => (
          <span
            key={i}
            className={`h-3.5 w-3.5 rounded-full border transition-colors duration-300 ${i < count ? palette.filled : palette.empty}`}
          />
        ))}
      </div>
      <span className={`text-xs uppercase tracking-wider opacity-70 ${palette.text}`}>
        {label} {count}/{max}
      </span>
    </div>
  );
}

function MatchHistory({ results }: { results: DraftRunState["results"] }) {
  if (results.length === 0) return null;
  return (
    <div className="flex flex-col items-center gap-2">
      <span className="text-[0.62rem] font-medium uppercase tracking-[0.18em] text-white/30">Match Log</span>
      <div className="flex items-center gap-1">
        {results.map((r, i) => (
          <div
            key={r.gameId}
            className={`flex h-7 w-7 items-center justify-center rounded-md text-[11px] font-bold ${
              r.result === "win"
                ? "bg-emerald-500/18 text-emerald-300"
                : r.result === "loss"
                  ? "bg-red-500/18 text-red-300"
                  : "bg-slate-500/18 text-slate-300"
            }`}
            title={`Match ${i + 1}: ${r.result}`}
          >
            {r.result === "win" ? "W" : r.result === "loss" ? "L" : "D"}
          </div>
        ))}
      </div>
    </div>
  );
}

// ── Main Component ────────────────────────────────────────────────────

export function DraftPage() {
  const phase = useDraftStore((s) => s.phase);
  const reset = useDraftStore((s) => s.reset);
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const [hoveredCard, setHoveredCard] = useState<CardHoverInfo | null>(null);
  const [introDismissed, setIntroDismissed] = useState(false);
  const [resumeLoading, setResumeLoading] = useState(false);

  useEffect(() => {
    if (searchParams.get("resume") !== "1") return;
    let cancelled = false;

    async function doResume() {
      setResumeLoading(true);
      try {
        await useDraftStore.getState().resumeDraft();
        if (!cancelled) setIntroDismissed(true);
      } catch {
        await useDraftStore.getState().abandonDraft();
      } finally {
        if (!cancelled) setResumeLoading(false);
      }
    }
    doResume();
    return () => { cancelled = true; };
  }, [searchParams]);

  useEffect(() => {
    return () => {
      reset();
    };
  }, [reset]);

  const handleStartDraft = useCallback(
    async (setCode: string, setName: string) => {
      const { difficulty, startDraft } = useDraftStore.getState();

      const resp = await fetch(__DRAFT_POOLS_URL__);
      if (!resp.ok) throw new Error(`Failed to load draft pools: ${resp.status}`);
      const allPools: Record<string, unknown> = await resp.json();
      const setPool = allPools[setCode.toLowerCase()] ?? allPools[setCode.toUpperCase()];
      if (!setPool) throw new Error(`No pool data for set: ${setCode}`);

      await startDraft(JSON.stringify(setPool), setCode, setName, difficulty);
    },
    [],
  );

  const handleLaunchMatch = useCallback(async () => {
    await useDraftStore.getState().launchMatch(navigate);
  }, [navigate]);

  const handleLaunchNextMatch = useCallback(async () => {
    await useDraftStore.getState().launchNextMatch(navigate);
  }, [navigate]);

  const handleEndRun = useCallback(async () => {
    await useDraftStore.getState().endRun();
    navigate("/draft");
  }, [navigate]);

  return (
    <div className="menu-scene relative flex min-h-screen flex-col overflow-hidden">
      <ScreenChrome onBack={() => navigate("/draft")} />
      {phase === "drafting" && introDismissed && (
        <CardPreview cardName={hoveredCard?.name ?? null} sourcePrinting={hoveredCard?.sourcePrinting} />
      )}

      <div className="relative z-10 mx-auto flex w-full max-w-6xl flex-col px-6 py-16">
        {resumeLoading ? (
          <div className="flex items-center justify-center py-24">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-gray-500 border-t-white" />
          </div>
        ) : (
          <div className="mb-12">
            <DraftSteps phase={phase} />
          </div>
        )}

        {!resumeLoading && phase === "setup" && (
          <div className="mx-auto w-full max-w-4xl">
            <h1 className="mb-8 menu-display text-3xl text-white">Quick Draft</h1>
            <SetSelector onStartDraft={handleStartDraft} />
          </div>
        )}

        {phase === "drafting" && !introDismissed && (
          <DraftIntro mode="quick" onContinue={() => setIntroDismissed(true)} />
        )}

        {phase === "drafting" && introDismissed && (
          <div className="flex gap-4">
            <div className="flex-1">
              <div className="mb-4">
                <DraftProgress />
              </div>
              <PackDisplay onCardHover={setHoveredCard} showAutoPick />
            </div>
            <PoolPanel onCardHover={setHoveredCard} />
          </div>
        )}

        {phase === "deckbuilding" && (
          <LimitedDeckBuilder />
        )}

        {phase === "launching" && (
          <FormatPicker onLaunch={handleLaunchMatch} />
        )}

        {!resumeLoading && phase === "playing" && (
          <BetweenMatches onNext={handleLaunchNextMatch} onEnd={handleEndRun} />
        )}

        {!resumeLoading && phase === "complete" && (
          <RunComplete onDone={handleEndRun} />
        )}
      </div>
    </div>
  );
}
