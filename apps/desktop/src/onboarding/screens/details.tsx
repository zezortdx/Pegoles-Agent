import { useState } from "react";

/**
 * "Technical details": Core's own words for people who want them (or for
 * a bug report), folded away by default and copyable in one press.
 */
export function TechnicalDetails({ text, label = "Technical details" }: { readonly text: string; readonly label?: string }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };
  return (
    <details className="ob-details">
      <summary className="ob-details__summary">{label}</summary>
      <pre className="ob-details__body">{text}</pre>
      <button type="button" className="btn btn--small btn--line" onClick={() => void copy()}>
        {copied ? "Copied" : "Copy details"}
      </button>
      <span className="visually-hidden" role="status">{copied ? "Copied to the clipboard" : ""}</span>
    </details>
  );
}
