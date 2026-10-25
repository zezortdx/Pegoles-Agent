import { useId, useState } from "react";
import { PegolesPresence } from "../../presence";
import { ShieldIcon, StopIcon, ModelIcon } from "../../ui/icons";
import { FlowDiagram } from "../visuals";
import type { ScreenProps } from "./types";

/** Screen 1: who Pegoles is, in one breath. */
export function WelcomeScreen({ headingRef, go, computer, animated }: ScreenProps) {
  const [open, setOpen] = useState(false);
  const panel = useId();
  return (
    <section className="ob-screen ob-screen--welcome" aria-labelledby="ob-title">
      <div className="ob-hero-mark">
        <PegolesPresence mode="idle" size={124} interactive={animated} pressable={false} field={false} decorative />
      </div>
      <h1 id="ob-title" ref={headingRef} tabIndex={-1} className="ob-title ob-title--hero">Welcome to Pegoles</h1>
      <p className="ob-tagline">AI gets its own computer.<br /><strong>Not your computer.</strong></p>
      <p className="ob-lead">
        Pegoles works inside an isolated computer of its own, so it can do tasks for you without directly controlling your personal {computer === "Mac" ? "Mac" : "desktop"}.
      </p>
      <div className="ob-actions ob-actions--center">
        <button type="button" className="btn btn--primary ob-cta" onClick={() => go("how")}>Get started</button>
        <button type="button" className="btn btn--quiet" aria-expanded={open} aria-controls={panel} onClick={() => setOpen((value) => !value)}>
          Learn how Pegoles works
        </button>
      </div>
      {open && (
        <div id={panel} className="ob-explainer" role="region" aria-label="How Pegoles works">
          <p>When you give Pegoles a task, a small AI model on this {computer} looks at Pegoles’ own screen and decides the next step: a click, some typing, a scroll.</p>
          <p>Every step is checked by fixed safety rules before it runs, and it only ever happens inside Pegoles’ isolated computer. It can’t see or touch your files, apps or screen, and its computer has no internet.</p>
          <p>You can watch every step, and stop a task at any moment.</p>
        </div>
      )}
    </section>
  );
}

/** Screen 2: how it works, as a picture and three promises. */
export function HowScreen({ headingRef, go, computer }: ScreenProps) {
  return (
    <section className="ob-screen" aria-labelledby="ob-title">
      <h1 id="ob-title" ref={headingRef} tabIndex={-1} className="ob-title">How Pegoles works</h1>
      <p className="ob-lead">You describe a task. Pegoles does it on its own computer while you watch.</p>
      <FlowDiagram computer={computer} />
      <ul className="ob-cards" aria-label="What that means for you">
        <li className="ob-card">
          <span className="ob-card__icon" aria-hidden="true"><ModelIcon size={18} /></span>
          <span className="ob-card__title">Local by default</span>
          <span className="ob-card__body">The included AI model runs on this {computer}. No account and no API key.</span>
        </li>
        <li className="ob-card">
          <span className="ob-card__icon" aria-hidden="true"><ShieldIcon size={18} /></span>
          <span className="ob-card__title">Isolated</span>
          <span className="ob-card__body">Tasks happen inside Pegoles’ own virtual computer, with no access to your files or the internet.</span>
        </li>
        <li className="ob-card">
          <span className="ob-card__icon" aria-hidden="true"><StopIcon size={18} /></span>
          <span className="ob-card__title">You’re in control</span>
          <span className="ob-card__body">Watch every step, and stop a task at any moment.</span>
        </li>
      </ul>
      <div className="ob-actions">
        <button type="button" className="btn btn--quiet" onClick={() => go("welcome")}>Back</button>
        <span className="ob-actions__spacer" />
        <button type="button" className="btn btn--primary ob-cta" onClick={() => go("check")}>Continue</button>
      </div>
    </section>
  );
}
