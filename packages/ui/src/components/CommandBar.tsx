import {
  useCallback,
  useId,
  useState,
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
} from "react";
import { m } from "motion/react";
import { radiusRole } from "../tokens/radius.js";
import { spring } from "../tokens/motion.js";
import { useFluxGlass } from "../runtime/context.js";
import { GlassButton } from "./GlassButton.js";
import { useGlassSurface } from "./GlassSurface.js";
import { ArrowUpIcon } from "./icons.js";
import { cx } from "./cx.js";
import "./command.css";

/** Shared layoutId: CommandBar and TaskController morph into each other. */
export const COMMAND_SURFACE_LAYOUT_ID = "pegoles-command-surface";

export interface CommandBarProps {
  /** Called with the trimmed text on Enter / send. */
  readonly onSubmit: (text: string) => void;
  /** Controlled value. Omit for an uncontrolled bar (cleared after submit). */
  readonly value?: string;
  readonly defaultValue?: string;
  readonly onValueChange?: (value: string) => void;
  readonly placeholder?: string;
  /** Accessible name of the form and the field. */
  readonly label?: string;
  readonly disabled?: boolean;
  /** A submission is in flight: blocks re-submit, shows a working send button. */
  readonly isBusy?: boolean;
  /** Leading slot — typically the PegolesMark. Decorative. */
  readonly leading?: ReactNode;
  /** Keyboard hint shown while the bar is idle, e.g. "⌘K". */
  readonly shortcutHint?: string;
  readonly autoFocus?: boolean;
  readonly inputRef?: Ref<HTMLInputElement>;
  /** Morph identity; share it with the TaskController that replaces this bar. */
  readonly layoutId?: string;
  readonly className?: string;
}

/**
 * The signature surface: a floating glass command field. Keyboard-first
 * (Enter sends, Escape clears then blurs, IME-safe), with an edge that
 * lights up while Pegoles is listening (focus). No open/close animation:
 * it is summoned too often for motion to be anything but delay.
 */
export function CommandBar({
  onSubmit,
  value: controlledValue,
  defaultValue = "",
  onValueChange,
  placeholder = "Ask Pegoles…",
  label = "Ask Pegoles",
  disabled = false,
  isBusy = false,
  leading,
  shortcutHint,
  autoFocus = false,
  inputRef,
  layoutId = COMMAND_SURFACE_LAYOUT_ID,
  className,
}: CommandBarProps) {
  const { transitionStyle } = useFluxGlass();
  const glass = useGlassSurface("regular", "CommandBar");
  const [internal, setInternal] = useState(defaultValue);
  const isControlled = controlledValue !== undefined;
  const value = isControlled ? controlledValue : internal;
  const canSubmit = value.trim().length > 0 && !disabled && !isBusy;
  const inputId = useId();

  const update = useCallback(
    (next: string) => {
      if (!isControlled) setInternal(next);
      onValueChange?.(next);
    },
    [isControlled, onValueChange],
  );

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit) return;
    onSubmit(value.trim());
    if (!isControlled) setInternal("");
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter" && event.nativeEvent.isComposing) {
      event.preventDefault();
      return;
    }
    if (event.key === "Escape") {
      if (value.length > 0) {
        event.preventDefault();
        update("");
      } else {
        event.currentTarget.blur();
      }
    }
  };

  return (
    <m.form
      ref={glass.ref}
      layoutId={transitionStyle === "morph" ? layoutId : undefined}
      transition={{ layout: spring.morph }}
      style={{ borderRadius: radiusRole.command }}
      className={cx("pg-glass", "pg-command", className)}
      {...glass.attributes}
      data-elevation="3"
      data-radius="xxl"
      data-busy={isBusy ? "true" : undefined}
      aria-label={label}
      aria-busy={isBusy || undefined}
      onSubmit={handleSubmit}
    >
      <m.div className="pg-command__row" layout="position">
        {leading && (
          <span className="pg-command__leading" aria-hidden="true">
            {leading}
          </span>
        )}
        <label className="pg-visually-hidden" htmlFor={inputId}>
          {label}
        </label>
        <input
          id={inputId}
          ref={inputRef}
          className="pg-command__input"
          type="text"
          value={value}
          placeholder={placeholder}
          disabled={disabled}
          autoFocus={autoFocus}
          autoComplete="off"
          autoCorrect="off"
          spellCheck
          enterKeyHint="send"
          onChange={(event: ChangeEvent<HTMLInputElement>) => update(event.target.value)}
          onKeyDown={handleKeyDown}
        />
        {shortcutHint && value.length === 0 && (
          <kbd className="pg-kbd pg-command__hint" aria-hidden="true">
            {shortcutHint}
          </kbd>
        )}
        <GlassButton
          type="submit"
          variant="primary"
          iconOnly
          icon={<ArrowUpIcon />}
          isLoading={isBusy}
          disabled={!canSubmit && !isBusy}
          className="pg-command__send"
        >
          Send to Pegoles
        </GlassButton>
      </m.div>
    </m.form>
  );
}
