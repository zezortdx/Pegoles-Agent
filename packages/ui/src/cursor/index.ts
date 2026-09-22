export {
  AgentCursorController,
  CLICK_MS,
  MAX_TRAIL_ELEMENTS,
  RM_FADE_MS,
  TRAIL_BY_TIER,
  TRAIL_LAG_MS,
  trailLengthFor,
  type AgentCursorControllerOptions,
  type AgentCursorElements,
  type CursorTimers,
  type FrameScheduler,
  type CursorPoint,
} from "./AgentCursorController.js";
export { AgentCursorLayer, type AgentCursorHandle, type AgentCursorLayerProps } from "./AgentCursorLayer.js";
export {
  AgentCursorOverlay,
  applyAgentCursorAction,
  productionAgentCursorSource,
  useAgentCursorSource,
  type AgentCursorAction,
  type AgentCursorOverlayProps,
  type AgentCursorSource,
  type AgentCursorSourceBinding,
} from "./agentCursorSource.js";
export {
  CURSOR_STATES,
  canTransition,
  transitionCursor,
  type CursorEvent,
  type CursorState,
} from "./cursorMachine.js";
export {
  AGENT_CURSOR_SPRING,
  SETTLE_DISTANCE,
  SETTLE_SPEED,
  isSettled,
  stepSpring,
  type SpringAxis,
  type SpringConfig,
} from "./spring.js";
