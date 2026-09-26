import { useCallback, useEffect, useRef, useState } from "react";
import { api, type ModelSettings } from "../lib/tauri";
import { errorText } from "./errors";

/** What the Model settings are doing: reading, saving or removing the key, or changing model and effort. */
export type ModelOp = "load" | "key" | "clear" | "choice";

export interface ModelSettingsError {
  readonly op: ModelOp;
  /** Core's own words (never contains the key). */
  readonly text: string;
}

export interface ModelSettingsState {
  readonly settings: ModelSettings | null;
  readonly pending: ModelOp | null;
  readonly error: ModelSettingsError | null;
  /**
   * Ask Core for the key: it opens a macOS dialog, so the key never passes
   * through this web view, and stores it in the Keychain. Resolves true
   * when Core stored one.
   */
  readonly enterKey: () => Promise<boolean>;
  readonly clearKey: () => Promise<boolean>;
  readonly choose: (model: string, effort: string) => Promise<boolean>;
}

/**
 * The model Pegoles works with, as Core reports it. The API key never
 * reaches this web view: Core asks for it in a native dialog and keeps it
 * in the Keychain. `onChanged` lets Core's status catch up after a change.
 */
export function useModelSettings(enabled: boolean, onChanged?: () => void): ModelSettingsState {
  const [settings, setSettings] = useState<ModelSettings | null>(null);
  const [pending, setPending] = useState<ModelOp | null>(null);
  const [error, setError] = useState<ModelSettingsError | null>(null);
  const alive = useRef(false);
  const busy = useRef(false);
  const changed = useRef(onChanged);

  useEffect(() => { changed.current = onChanged; }, [onChanged]);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);

  const perform = useCallback(async (op: ModelOp, command: () => Promise<ModelSettings>): Promise<boolean> => {
    if (busy.current) return false;
    busy.current = true;
    setPending(op);
    setError(null);
    try {
      const next = await command();
      if (alive.current) setSettings(next);
      if (op !== "load") changed.current?.();
      return true;
    } catch (raw) {
      if (alive.current) setError({ op, text: errorText(raw).trim() || "No details were reported." });
      return false;
    } finally {
      busy.current = false;
      if (alive.current) setPending(null);
    }
  }, []);

  useEffect(() => {
    if (enabled) void perform("load", api.getModelSettings);
  }, [enabled, perform]);

  const enterKey = useCallback(() => perform("key", api.enterApiKey), [perform]);
  const clearKey = useCallback(() => perform("clear", api.clearApiKey), [perform]);
  const choose = useCallback((model: string, effort: string) => perform("choice", () => api.setModelSettings(model, effort)), [perform]);

  return { settings, pending, error, enterKey, clearKey, choose };
}
