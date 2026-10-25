import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, m } from "motion/react";
import { api, type OnboardingStep, type StatusPayload, type SystemCheck } from "../lib/tauri";
import { duration, ease } from "../lib/motion";
import { errorText } from "../state/errors";
import type { IntelligenceState } from "../state/useIntelligence";
import type { LocalView } from "../state/localModel";
import { computerWord } from "./systemCheck";
import { useSetupRun } from "./useSetupRun";
import { HowScreen, WelcomeScreen } from "./screens/intro";
import { CheckScreen } from "./screens/check";
import { SetupScreen } from "./screens/setup";
import { IntelligenceScreen, ReadyScreen } from "./screens/finish";
import "./onboarding.css";

const ORDER: readonly OnboardingStep[] = ["welcome", "how", "check", "setup", "intelligence", "ready"];
const STEP_NAME: Record<OnboardingStep, string> = {
  welcome: "Welcome", how: "How it works", check: "System check", setup: "Setup", intelligence: "Intelligence", ready: "First task",
};

export interface OnboardingProps {
  readonly initialStep: OnboardingStep;
  readonly resumedAfterRestart: boolean;
  readonly status: StatusPayload | null;
  readonly intelligence: IntelligenceState;
  readonly local: LocalView;
  readonly animated: boolean;
  readonly refresh: () => Promise<void>;
  /** Onboarding is over; `prompt` is the first task to start, if chosen. */
  readonly onFinish: (prompt: string | null) => Promise<void>;
}

/**
 * First run, full window: six short screens from "what is this" to a first
 * task. The step is saved in Core as the person moves, so a restart (for
 * example the one Windows needs to turn virtualization on) resumes here.
 */
export function Onboarding(props: OnboardingProps) {
  const { initialStep, status, intelligence, local, animated, refresh, onFinish } = props;
  const [step, setStep] = useState<OnboardingStep>(initialStep);
  const [check, setCheck] = useState<SystemCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkFailure, setCheckFailure] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const first = useRef(true);
  const run = useSetupRun({ status, local, intelligence, refresh });

  const computer = check ? computerWord(check.platform) : /win/i.test(navigator.userAgent) ? "PC" : "Mac";

  const go = useCallback((next: OnboardingStep) => {
    setStep(next);
    void api.setOnboardingStep(next).catch(() => undefined);
  }, []);

  const recheck = useCallback(() => {
    setChecking(true);
    setCheckFailure(null);
    api.systemCheck()
      .then((result) => setCheck(result))
      .catch((error: unknown) => setCheckFailure(errorText(error)))
      .finally(() => setChecking(false));
  }, []);

  // The check runs when its screen opens (and again on request).
  useEffect(() => { if (step === "check") recheck(); }, [step, recheck]);

  // Each screen's title takes focus, so screen readers hear where they are.
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    window.setTimeout(() => heading.current?.focus({ preventScroll: true }), 0);
  }, [step]);

  const finish = async (prompt: string | null) => {
    setFinishing(true);
    try { await onFinish(prompt); } finally { setFinishing(false); }
  };

  const index = ORDER.indexOf(step);
  const screen = { headingRef: heading, go, computer, animated };
  const enter = animated ? { opacity: 0, y: 10 } : { opacity: 0 };
  return (
    <div className="onboarding" data-step={step}>
      <div className="onboarding__drag" data-tauri-drag-region aria-hidden="true" />
      <nav className="ob-progressdots" aria-label="Setup progress">
        <ol>
          {ORDER.map((name, i) => (
            <li key={name} data-state={i < index ? "done" : i === index ? "current" : "todo"} aria-current={i === index ? "step" : undefined}>
              <span className="visually-hidden">{STEP_NAME[name]}{i < index ? ", done" : i === index ? ", current" : ""}</span>
            </li>
          ))}
        </ol>
        <span className="ob-progressdots__label" aria-hidden="true">Step {index + 1} of {ORDER.length}</span>
      </nav>
      <main className="onboarding__stage">
        <AnimatePresence mode="wait" initial={false}>
          <m.div
            key={step}
            className="onboarding__screen"
            initial={enter}
            animate={{ opacity: 1, y: 0, transition: { duration: duration.scene, ease: ease.out } }}
            exit={{ opacity: 0, transition: { duration: duration.micro, ease: ease.exit } }}
          >
            {step === "welcome" && <WelcomeScreen {...screen} />}
            {step === "how" && <HowScreen {...screen} />}
            {step === "check" && (
              <CheckScreen {...screen} check={check} checking={checking} failure={checkFailure} onRecheck={recheck}
                resumedAfterRestart={props.resumedAfterRestart} />
            )}
            {step === "setup" && <SetupScreen {...screen} run={run} local={local} status={status} />}
            {step === "intelligence" && <IntelligenceScreen {...screen} intelligence={intelligence} local={local} />}
            {step === "ready" && <ReadyScreen {...screen} onFinish={(prompt) => void finish(prompt)} finishing={finishing} />}
          </m.div>
        </AnimatePresence>
      </main>
    </div>
  );
}
