import type { Ref } from "react";
import type { OnboardingStep } from "../../lib/tauri";

/** What every onboarding screen gets from the container. */
export interface ScreenProps {
  readonly headingRef: Ref<HTMLHeadingElement>;
  readonly go: (step: OnboardingStep) => void;
  /** "Mac" or "PC". */
  readonly computer: string;
  readonly animated: boolean;
}
