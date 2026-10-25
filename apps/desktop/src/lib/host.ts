/** "Mac" or "PC", for sentences about the machine people are on. */
export const HOST: "Mac" | "PC" =
  typeof navigator !== "undefined" && /windows/i.test(navigator.userAgent) ? "PC" : "Mac";

/** Cloud planners need a native confirmation window, which exists on macOS only so far. */
export const CLOUD_AVAILABLE = HOST === "Mac";
