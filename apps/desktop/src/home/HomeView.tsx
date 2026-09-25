import type { ReactNode, Ref } from "react";
import { m } from "motion/react";
import { PegolesPresence, type PresenceMode } from "../presence";
import { duration, ease } from "../lib/motion";
import { FolderIcon, GlobeIcon, TerminalIcon } from "../ui/icons";

/** Pegoles' mark travels from Home into the task's status bar. */
export const PRESENCE_LAYOUT_ID = "pegoles-presence";

export interface Starter {
  readonly label: string;
  readonly prompt: string;
  readonly icon: ReactNode;
}

export const STARTERS: readonly Starter[] = [
  { label: "Research something on the web", prompt: "Research the three most popular note-taking apps and summarize how their pricing compares", icon: <GlobeIcon size={15} /> },
  { label: "Organize a folder", prompt: "Sort the files in Downloads into folders by type and list anything that looks like a duplicate", icon: <FolderIcon size={15} /> },
  { label: "Write and test a script", prompt: "Write a Python script that renames photos by the date they were taken, then test it on a sample folder", icon: <TerminalIcon size={15} /> },
];

export interface HomeViewProps {
  readonly presence: PresenceMode;
  readonly headingRef?: Ref<HTMLHeadingElement>;
  /** The composer has focus or text: Pegoles looks toward it. */
  readonly attentive: boolean;
  readonly nudge: number;
  /** No tasks yet: offer a few real jobs to start from. */
  readonly firstRun: boolean;
  readonly onStarter: (prompt: string) => void;
  readonly onPressPresence: () => void;
  readonly animated: boolean;
}

/**
 * A work surface, not a landing page: Pegoles, one question, and (the
 * first time) a few real jobs to start from. The composer below is the
 * primary action; everything else stays quiet.
 */
export function HomeView({ presence, headingRef, attentive, nudge, firstRun, onStarter, onPressPresence, animated }: HomeViewProps) {
  const enter = (delay: number) => animated
    ? { initial: { opacity: 0, y: 6 }, animate: { opacity: 1, y: 0, transition: { duration: duration.surface, ease: ease.out, delay } } }
    : {};
  return (
    <section className="home" aria-labelledby="home-title">
      <div className="home__center">
        <m.div className="home__mark" layoutId={PRESENCE_LAYOUT_ID} data-thought-anchor="presence">
          <PegolesPresence mode={presence} size={52} look={attentive ? { x: 0, y: 1 } : null} nudge={nudge}
            interactive pressable onPress={onPressPresence} field={false} />
        </m.div>
        <m.h1 id="home-title" ref={headingRef} tabIndex={-1} className="home__title" {...enter(0.04)}>What should Pegoles do?</m.h1>
        <m.p className="home__lead" {...enter(0.08)}>It works on its own computer. Watch, step in or take over whenever you like.</m.p>
        {firstRun && (
          <m.ul className="starters" aria-label="A few places to start" {...enter(0.12)}>
            {STARTERS.map((starter) => (
              <li key={starter.label}>
                <button type="button" className="starter" onClick={() => onStarter(starter.prompt)}>
                  <span className="starter__icon" aria-hidden="true">{starter.icon}</span>
                  <span className="starter__label">{starter.label}</span>
                </button>
              </li>
            ))}
          </m.ul>
        )}
      </div>
    </section>
  );
}
