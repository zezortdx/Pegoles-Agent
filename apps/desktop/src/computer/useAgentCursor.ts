import { useMemo, useState, type SyntheticEvent } from "react";
import { agentCursorSource } from "../lib/agentCursorFeed";

type FrameSize = { readonly width: number; readonly height: number };

/**
 * The size of the picture actually on screen (its natural size), so the
 * agent cursor maps into exactly what is drawn. Only changes when the guest
 * resolution does: one render, not one per snapshot.
 */
export function useFrameSize(fallback: FrameSize | null) {
  const [natural, setNatural] = useState<FrameSize | null>(null);
  const onLoad = (event: SyntheticEvent<HTMLImageElement>) => {
    const { naturalWidth: width, naturalHeight: height } = event.currentTarget;
    if (!(width > 0 && height > 0)) return;
    setNatural((previous) => (previous?.width === width && previous.height === height ? previous : { width, height }));
  };
  return { frameSize: natural ?? fallback, onLoad };
}

/** The agent cursor feed for one computer: a new computer gets a new feed and a fresh cursor. */
export function useCursorSource(computerId: string | null | undefined) {
  return useMemo(() => (computerId ? agentCursorSource(computerId) : null), [computerId]);
}
