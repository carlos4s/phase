import { menuButtonClass } from "../menu/buttonStyles";

// ── Types ───────────────────────────────────────────────────────────────

type DraftMode = "quick" | "pod";

interface DraftIntroProps {
  mode: DraftMode;
  podSize?: number;
  onContinue: () => void;
}

// ── Steps ───────────────────────────────────────────────────────────────

interface Step {
  icon: string;
  text: string;
}

const QUICK_STEPS: Step[] = [
  { icon: "1", text: "You'll open 3 packs of 14 cards each" },
  { icon: "2", text: "Pick one card per pack, then pass the rest to AI drafters" },
  { icon: "3", text: "Packs alternate direction each round — left, right, left" },
  { icon: "4", text: "After all picks, build a 40-card deck and play a match" },
];

function podSteps(podSize: number): Step[] {
  return [
    { icon: "1", text: `You're drafting with ${podSize} players in a pod` },
    { icon: "2", text: "Open 3 packs of 14 cards — pick one, pass the rest" },
    { icon: "3", text: "Packs alternate direction each round — left, right, left" },
    { icon: "4", text: "After drafting, build a 40-card deck and play tournament matches" },
  ];
}

// ── Component ───────────────────────────────────────────────────────────

export function DraftIntro({ mode, podSize = 8, onContinue }: DraftIntroProps) {
  const steps = mode === "quick" ? QUICK_STEPS : podSteps(podSize);
  const title = mode === "quick" ? "Quick Draft" : "Pod Draft";

  return (
    <div className="mx-auto flex w-full max-w-lg flex-col items-center gap-8 py-12">
      <div className="flex flex-col items-center gap-2">
        <h1 className="menu-display text-3xl text-white">{title}</h1>
        <p className="text-sm text-white/50">Here's how it works</p>
      </div>

      <div className="flex w-full flex-col gap-3">
        {steps.map((step) => (
          <div
            key={step.icon}
            className="flex items-start gap-4 rounded-[16px] border border-white/10 bg-black/18 px-5 py-4 backdrop-blur-md"
          >
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-white/15 bg-white/8 text-sm font-semibold text-white/70">
              {step.icon}
            </span>
            <span className="pt-0.5 text-sm leading-relaxed text-white/80">
              {step.text}
            </span>
          </div>
        ))}
      </div>

      <button
        onClick={onContinue}
        className={menuButtonClass({ tone: "emerald", size: "lg" })}
      >
        Start Drafting
      </button>
    </div>
  );
}
