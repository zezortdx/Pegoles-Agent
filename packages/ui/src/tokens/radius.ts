/**
 * Pegoles Flux Glass — corner radii (px).
 *
 * Concentric rule: an inner radius equals the outer radius minus the
 * padding between them (viewport frame 24 − 12 inset = slot 12). Controls
 * are capsules; floating surfaces share one radius so the CommandBar →
 * TaskController morph only changes size, never corner language.
 */
export const radius = {
  xs: 4,
  sm: 8,
  md: 12,
  lg: 16,
  xl: 20,
  xxl: 24,
  pill: 9999,
} as const;

export type RadiusToken = keyof typeof radius;

export const radiusRole = {
  /** Keycaps, tiny chips. */
  key: radius.xs,
  /** Buttons, status pills, segmented controls. */
  control: radius.pill,
  /** Text fields and rows. */
  field: radius.md,
  /** Cards, panels, glass surfaces. */
  surface: radius.xl,
  /** CommandBar and TaskController (shared for the morph). */
  command: radius.xxl,
  /** ComputerViewport outer frame. */
  viewport: radius.xxl,
  /**
   * The framebuffer slot. The native VZVirtualMachineView host view MUST
   * clip to this exact radius so the webview edge ring lines up with it.
   */
  viewportSlot: radius.md,
} as const;

export type RadiusRole = keyof typeof radiusRole;
