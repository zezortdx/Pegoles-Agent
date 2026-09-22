import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { cx } from "./cx.js";
import "./button.css";

export type GlassButtonVariant = "primary" | "secondary" | "quiet";
export type GlassButtonSize = "sm" | "md" | "lg";

export interface GlassButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** primary: the one main action · secondary: glass · quiet: text-weight. */
  readonly variant?: GlassButtonVariant;
  readonly size?: GlassButtonSize;
  /** Leading icon (decorative; the label names the action). */
  readonly icon?: ReactNode;
  /** Trailing adornment, e.g. a keyboard hint. */
  readonly trailing?: ReactNode;
  /** Shows a working indicator and blocks presses; the label stays for width stability. */
  readonly isLoading?: boolean;
  /**
   * Icon-only button. `children` is then rendered visually hidden and
   * still names the button for assistive tech.
   */
  readonly iconOnly?: boolean;
  /** Neutral/white emphasis (e.g. "Return to Pegoles" while user controls). */
  readonly tone?: "accent" | "neutral";
}

/**
 * Capsule button. Feedback on press (scale 0.97 in 100 ms; a brightness
 * change instead under reduced motion), designed hover (fine pointers
 * only), cyan focus ring, and a disabled state that still reads.
 * No backdrop-filter: buttons live on surfaces, never blur again.
 */
export const GlassButton = forwardRef<HTMLButtonElement, GlassButtonProps>(function GlassButton(
  {
    variant = "secondary",
    size = "md",
    icon,
    trailing,
    isLoading = false,
    iconOnly = false,
    tone = "accent",
    type = "button",
    disabled,
    className,
    children,
    ...rest
  },
  ref,
) {
  const isDisabled = disabled === true || isLoading;
  return (
    <button
      ref={ref}
      type={type}
      className={cx("pg-button", className)}
      data-variant={variant}
      data-size={size}
      data-tone={tone}
      data-icon-only={iconOnly ? "true" : undefined}
      data-loading={isLoading ? "true" : undefined}
      disabled={isDisabled}
      aria-busy={isLoading || undefined}
      {...rest}
    >
      {isLoading ? (
        <span className="pg-button__spinner pg-work-anim" aria-hidden="true" />
      ) : (
        icon && (
          <span className="pg-button__icon" aria-hidden="true">
            {icon}
          </span>
        )
      )}
      {iconOnly ? (
        <span className="pg-visually-hidden">{children}</span>
      ) : (
        children !== undefined && <span className="pg-button__label">{children}</span>
      )}
      {trailing && !iconOnly && <span className="pg-button__trailing">{trailing}</span>}
    </button>
  );
});
