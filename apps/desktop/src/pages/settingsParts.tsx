import type { ReactNode } from "react";

/** One labelled row of a settings group. */
export function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="setting" role="listitem">
      <div className="setting__text">
        <span className="setting__label">{label}</span>
        {hint && <span className="setting__hint">{hint}</span>}
      </div>
      <div className="setting__value">{children}</div>
    </div>
  );
}

/** A titled group of rows on one matte surface, macOS grouped-form style. */
export function Section({ id, title, lead, note, after, children }: {
  id: string; title: string; lead?: string; note?: string; after?: ReactNode; children: ReactNode;
}) {
  return (
    <section className="settings__section" aria-labelledby={id} data-section={id}>
      <div className="settings__head">
        <h2 id={id} className="settings__title" tabIndex={-1}>{title}</h2>
        {lead && <p className="settings__lead">{lead}</p>}
      </div>
      <div className="settings__group" role="list">{children}</div>
      {after}
      {note && <p className="settings__note">{note}</p>}
    </section>
  );
}
