export { PegolesMark, type PegolesMarkProps } from "./PegolesMark.js";
export {
  PRESENCE_STATES,
  PRESENCE_LABEL,
  SUCCESS_HOLD_MS,
  createPresenceMachine,
  isPresenceState,
  timedTransition,
  transitionPresence,
  type PresenceEvent,
  type PresenceEventType,
  type PresenceMachine,
  type PresenceState,
  type PresenceTimers,
} from "./presenceMachine.js";
export {
  EMPTY_PRESENCE_SNAPSHOT,
  derivePresence,
  presenceEventsForChange,
  type PresenceSnapshot,
  type TaskStatusWire,
  type ViewportStateWire,
} from "./presenceSource.js";
export { usePresence } from "./usePresence.js";
export {
  COMPACT_MARK_PX,
  ORBIT_PERIOD_MS,
  presenceVisual,
  type AnimationGate,
  type AnimationTarget,
  type PresenceAnimation,
  type PresenceLayers,
  type PresenceLevels,
  type PresenceVisual,
  type PresenceVisualInput,
} from "./presenceVisual.js";
export {
  DEFAULT_ATTENTION_DEG,
  EYE_SHIFT_PER_PX,
  MAX_EYE_SHIFT_PX,
  attentionAngle,
  eyeOffset,
  lookToward,
  type Vec2,
} from "./geometry.js";
