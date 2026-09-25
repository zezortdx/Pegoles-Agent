import { useCallback, useLayoutEffect, useState, type ReactNode, type Ref } from "react";
import { ArrowUpIcon } from "../ui/icons";

/** Core's task title contract: one line, at most 500 Unicode scalars. */
export const TITLE_LIMIT = 500;
const COUNT_FROM = 400;
const MAX_HEIGHT_PX = 220;

export function normalizeTitle(value: string): string {
  return value.replace(/[\r\n\t]+/g, " ").trim();
}

export function titleProblem(normalized: string): string | null {
  const chars = [...normalized];
  if (chars.length > TITLE_LIMIT) return `Keep it under ${TITLE_LIMIT} characters.`;
  const control = chars.some((char) => {
    const code = char.codePointAt(0) ?? 0;
    return code < 32 || (code >= 127 && code <= 159);
  });
  return control ? "Remove the control characters." : null;
}

export interface ComposerProps {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly onSubmit: (value: string) => void;
  readonly onFocusChange?: (focused: boolean) => void;
  /** Fires on each character typed (drives Pegoles' small reactions). */
  readonly onKeystroke?: () => void;
  readonly inputRef?: Ref<HTMLTextAreaElement>;
  readonly placeholder: string;
  readonly disabled?: boolean;
  readonly busy?: boolean;
  /** Error shown under the field, e.g. a failed hand-off. */
  readonly problem?: string | null;
  /** Where the job will run, attached above the field (its computer). */
  readonly strip?: ReactNode;
  /** How it will run, beside the send button (safety rules, model). Facts you can open, never fake choices. */
  readonly controls?: ReactNode;
}

/**
 * The one place work starts. The field and your words dominate; where the
 * job runs sits on a tab above it and how it runs sits quietly beside the
 * send button, brightening only when the composer has your attention.
 * Text wraps but stays one line for Core; Enter hands it over.
 */
export function Composer({
  value, onChange, onSubmit, onFocusChange, onKeystroke, inputRef, placeholder,
  disabled = false, busy = false, problem, strip, controls,
}: ComposerProps) {
  const [field, setField] = useState<HTMLTextAreaElement | null>(null);
  const [focused, setFocused] = useState(false);
  const normalized = normalizeTitle(value);
  const invalid = titleProblem(normalized);
  const length = [...normalized].length;
  const canSend = !disabled && !busy && length > 0 && !invalid;

  useLayoutEffect(() => {
    if (!field) return;
    field.style.height = "auto";
    field.style.height = `${Math.min(field.scrollHeight, MAX_HEIGHT_PX)}px`;
  }, [field, value]);

  const setRefs = useCallback((element: HTMLTextAreaElement | null) => {
    setField(element);
    if (typeof inputRef === "function") inputRef(element);
    else if (inputRef) (inputRef as { current: HTMLTextAreaElement | null }).current = element;
  }, [inputRef]);

  const focus = (next: boolean) => { setFocused(next); onFocusChange?.(next); };
  const message = invalid ?? problem ?? null;
  return (
    <div className="composer-shell" data-focused={focused || undefined} data-disabled={disabled || undefined}>
      {strip && <div className="composer__strip">{strip}</div>}
      <form
        className="composer"
        aria-label="New task"
        data-filled={length > 0 || undefined}
        data-focused={focused || undefined}
        data-busy={busy || undefined}
        data-invalid={invalid ? true : undefined}
        onSubmit={(event) => { event.preventDefault(); if (canSend) onSubmit(normalized); }}
        onMouseDown={(event) => {
          // Clicking the composer's padding still lands in the field.
          if (event.target === event.currentTarget) { event.preventDefault(); field?.focus(); }
        }}
      >
        <textarea
          ref={setRefs}
          className="composer__input"
          aria-label="New task"
          aria-invalid={invalid ? true : undefined}
          aria-describedby={message ? "composer-message" : undefined}
          rows={1}
          placeholder={placeholder}
          value={value}
          disabled={disabled}
          spellCheck
          onChange={(event) => {
            if (event.target.value.length > value.length) onKeystroke?.();
            onChange(event.target.value);
          }}
          onFocus={() => focus(true)}
          onBlur={() => focus(false)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              event.currentTarget.form?.requestSubmit();
            }
          }}
        />
        <div className="composer__row">
          {controls && <div className="composer__controls">{controls}</div>}
          <span className="composer__spacer" />
          {length >= COUNT_FROM && (
            <span className="composer__count mono" data-over={length > TITLE_LIMIT || undefined}>{length} / {TITLE_LIMIT}</span>
          )}
          <span className="composer__hint" aria-hidden="true">Return to hand over</span>
          <button type="submit" className="composer__send" data-ready={canSend || undefined} aria-label={busy ? "Handing over" : "Hand to Pegoles"} disabled={!canSend}>
            {busy ? <span className="spinner" aria-hidden="true" /> : <ArrowUpIcon size={16} />}
          </button>
        </div>
      </form>
      {message && <p id="composer-message" className="composer__message" role={invalid ? "status" : "alert"}>{message}</p>}
    </div>
  );
}
