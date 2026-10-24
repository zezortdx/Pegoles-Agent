import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { api } from "../lib/tauri";
import { activateLocale } from "../i18n";
import { DEFAULT_PREFERENCES, type Preferences } from "./types";

interface PreferencesState {
  readonly preferences: Preferences;
  readonly customizerOpen: boolean;
  readonly setCustomizerOpen: (open: boolean) => void;
  readonly ready: boolean;
  readonly loading: boolean;
  readonly loadError: boolean;
  readonly saving: boolean;
  readonly reload: () => Promise<void>;
  readonly save: (preferences: Preferences) => Promise<void>;
}
const PreferencesContext = createContext<PreferencesState>({
  customizerOpen: false, setCustomizerOpen: () => undefined,
  preferences: DEFAULT_PREFERENCES, ready: false, loading: false, loadError: false, saving: false,
  reload: async () => undefined,
  save: async () => { throw new Error("Desktop app required."); },
});
export function usePreferences(): PreferencesState { return useContext(PreferencesContext); }

export function PreferencesProvider({ children }: { readonly children: ReactNode }) {
  const [customizerOpen, setCustomizerOpen] = useState(false);
  const [preferences, setPreferences] = useState(DEFAULT_PREFERENCES);
  const [ready, setReady] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [saving, setSaving] = useState(false);
  const mounted = useRef(false);
  const inFlight = useRef(false);
  const generation = useRef(0);
  const reload = useCallback(async () => {
    if (!isTauri() || inFlight.current) return;
    const ticket = ++generation.current;
    setLoading(true); setLoadError(false); setReady(false);
    try {
      const loaded = await api.getPreferences();
      // Core validates the payload; guard absent transport replies in browser previews.
      if (!loaded || loaded.schema_version !== 1) throw new Error("Preferences unavailable.");
      if (!mounted.current || ticket !== generation.current) return;
      activateLocale(loaded.locale); setPreferences(loaded); setReady(true);
    } catch {
      if (mounted.current && ticket === generation.current) setLoadError(true);
    } finally {
      if (mounted.current && ticket === generation.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    activateLocale("en");
    void reload();
    const ticket = generation.current;
    return () => { mounted.current = false; generation.current = ticket + 1; activateLocale("en"); };
  }, [reload]);
  const save = useCallback(async (next: Preferences) => {
    if (!isTauri() || !ready || inFlight.current) throw new Error("Preferences unavailable.");
    inFlight.current = true; setSaving(true);
    try {
      const saved = await api.setPreferences(next);
      if (!saved || saved.schema_version !== 1) throw new Error("Preferences unavailable.");
      if (mounted.current) { activateLocale(saved.locale); setPreferences(saved); }
    } finally {
      inFlight.current = false;
      if (mounted.current) setSaving(false);
    }
  }, [ready]);
  return <PreferencesContext.Provider value={{ preferences, customizerOpen, setCustomizerOpen, ready, loading, loadError, saving, reload, save }}>{children}</PreferencesContext.Provider>;
}
