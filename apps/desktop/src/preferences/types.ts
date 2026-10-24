import type { ShapeId } from "../presence/engine/skins";
import type { ExpressionId } from "../presence/engine/expressions";

export type Locale = "en" | "pt-BR";
export const AVATAR_COLORS = ["silver", "graphite", "pearl"] as const;
export type AvatarColor = typeof AVATAR_COLORS[number];
export interface AvatarProfile {
  readonly shape: ShapeId;
  readonly color: AvatarColor;
  readonly expression: ExpressionId;
  readonly name: string;
}
export interface Preferences {
  readonly schema_version: 1;
  readonly avatar: AvatarProfile;
  readonly locale: Locale;
}
export const DEFAULT_PREFERENCES: Preferences = {
  schema_version: 1,
  avatar: { shape: "circle", color: "silver", expression: "attentive", name: "Pegoles" },
  locale: "en",
};
