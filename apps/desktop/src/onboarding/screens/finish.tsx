import { useState } from "react";
import { PegolesPresence } from "../../presence";
import type { IntelligenceState } from "../../state/useIntelligence";
import type { LocalView } from "../../state/localModel";
import { CheckIcon, ChevronDownIcon, TerminalIcon, FileIcon, ModelIcon } from "../../ui/icons";
import type { ScreenProps } from "./types";

export interface IntelligenceScreenProps extends ScreenProps {
  readonly intelligence: IntelligenceState;
  readonly local: LocalView;
}

/** Screen 5: Pegoles Local is the answer; cloud models stay folded away. */
export function IntelligenceScreen({ headingRef, go, computer, intelligence, local }: IntelligenceScreenProps) {
  const [busy, setBusy] = useState(false);
  const provider = intelligence.intelligence?.provider ?? "local";
  const continueLocal = async () => {
    setBusy(true);
    const ok = provider === "local" || await intelligence.setProvider("local");
    setBusy(false);
    if (ok) go("ready");
  };
  return (
    <section className="ob-screen" aria-labelledby="ob-title">
      <h1 id="ob-title" ref={headingRef} tabIndex={-1} className="ob-title">How Pegoles thinks</h1>
      <p className="ob-lead">
        {local.stage === "ready" ? "Pegoles comes with its own AI model. It’s already set up, and nothing else is needed." : "Pegoles comes with its own AI model, free and private. Nothing else is needed."}
      </p>
      <div className="ob-choice" role="group" aria-labelledby="ob-local-title" data-selected="true">
        <div className="ob-choice__head">
          <span className="ob-choice__icon" aria-hidden="true"><ModelIcon size={18} /></span>
          <span id="ob-local-title" className="ob-choice__title">Pegoles Local</span>
          <span className="ob-badge">Recommended</span>
          <span className="ob-choice__state" data-ready={local.stage === "ready" || undefined}>{local.stage === "ready" ? <><CheckIcon size={13} />Ready</> : "Not set up yet"}</span>
        </div>
        <ul className="ob-choice__facts">
          <li>Free</li><li>Private</li><li>Runs on this {computer}</li><li>No API key</li>
        </ul>
      </div>
      <details className="ob-details ob-details--cloud">
        <summary className="ob-details__summary">Cloud models <ChevronDownIcon size={12} /></summary>
        <div className="ob-details__text">
          <p>Pegoles can also use Anthropic’s Claude with your own API key. Cloud models can be more capable, but each step’s screenshot of Pegoles’ computer is sent to Anthropic.</p>
          <p>You can switch any time in Settings → Intelligence. More providers are planned.</p>
        </div>
      </details>
      {intelligence.error?.op === "provider" && <p className="ob-muted" role="alert">Couldn’t switch to Pegoles Local. {intelligence.error.text}</p>}
      <div className="ob-actions">
        <button type="button" className="btn btn--quiet" onClick={() => go("setup")}>Back</button>
        <span className="ob-actions__spacer" />
        <button type="button" className="btn btn--primary ob-cta" disabled={busy} onClick={() => void continueLocal()}>Continue with Pegoles Local</button>
      </div>
    </section>
  );
}

/**
 * First tasks that match what Pegoles' computer really has today: a Linux
 * desktop with a terminal, Python and text editors, and no internet.
 */
export const FIRST_TASKS = [
  { id: "file", title: "Create a text file", prompt: "Open the terminal and create a text file called hello.txt that says Hello from Pegoles, then show its contents.", icon: <FileIcon size={16} /> },
  { id: "math", title: "Work something out", prompt: "In the terminal, use Python to calculate 1234 × 5678 and tell me the answer.", icon: <TerminalIcon size={16} /> },
  { id: "note", title: "Write a short note", prompt: "Open the nano editor in the terminal, write a three-line note about planning a picnic, and save it as note.txt.", icon: <FileIcon size={16} /> },
] as const;

export interface ReadyScreenProps extends ScreenProps {
  readonly onFinish: (prompt: string | null) => void;
  readonly finishing: boolean;
}

/** Screen 6: ready, with a first task that shows it working within minutes. */
export function ReadyScreen({ headingRef, onFinish, finishing, animated }: ReadyScreenProps) {
  const [choice, setChoice] = useState<string>(FIRST_TASKS[0].id);
  const chosen = FIRST_TASKS.find((task) => task.id === choice) ?? FIRST_TASKS[0];
  return (
    <section className="ob-screen ob-screen--ready" aria-labelledby="ob-title">
      <div className="ob-hero-mark ob-hero-mark--ready">
        <PegolesPresence mode="done" size={96} interactive={animated} field={false} decorative />
      </div>
      <h1 id="ob-title" ref={headingRef} tabIndex={-1} className="ob-title ob-title--hero">Pegoles is ready</h1>
      <p className="ob-lead">Try a first task. Pegoles does it on its own computer while you watch, and you can stop it at any moment.</p>
      <fieldset className="ob-tasks">
        <legend className="visually-hidden">Choose a first task</legend>
        {FIRST_TASKS.map((task) => (
          <label key={task.id} className="ob-task" data-selected={task.id === choice || undefined}>
            <input type="radio" name="first-task" value={task.id} checked={task.id === choice} onChange={() => setChoice(task.id)} />
            <span className="ob-task__icon" aria-hidden="true">{task.icon}</span>
            <span className="ob-task__text">
              <span className="ob-task__title">{task.title}</span>
              <span className="ob-task__prompt">“{task.prompt}”</span>
            </span>
          </label>
        ))}
      </fieldset>
      <div className="ob-actions">
        <button type="button" className="btn btn--quiet" disabled={finishing} onClick={() => onFinish(null)}>I’ll write my own</button>
        <span className="ob-actions__spacer" />
        <button type="button" className="btn btn--primary ob-cta" disabled={finishing} onClick={() => onFinish(chosen.prompt)}>Try your first task</button>
      </div>
    </section>
  );
}
