import { t, useLocale } from "../i18n";
import { AnimatePresence, m } from "motion/react";
import type { HumanError } from "../state/errors";
import { duration, ease } from "../lib/motion";
import { CloseIcon } from "../ui/icons";

/** General problems only: task and computer errors live where they happened. */
export function Toast({ error, onDismiss }: { error: HumanError | null; onDismiss: () => void }) {
  useLocale();
  return (
    <AnimatePresence>
      {error && (
        <m.div
          key={error.title}
          className="toast"
          role="alert"
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0, transition: { duration: duration.layout, ease: ease.out } }}
          exit={{ opacity: 0, transition: { duration: duration.microSlow, ease: ease.exit } }}
        >
          <div className="toast__text">
            <p className="toast__title">{error.title}</p>
            {error.hint && <p className="toast__hint">{error.hint}</p>}
            <details className="details">
              <summary>{t("Details")}</summary>
              <pre className="diagnostic">{error.detail}</pre>
            </details>
          </div>
          <button type="button" className="icon-btn" aria-label={t("Dismiss")} onClick={onDismiss}><CloseIcon size={14} /></button>
        </m.div>
      )}
    </AnimatePresence>
  );
}
