import type { ReactNode, SVGProps } from "react";
import type { Capability } from "../lib/execution";
import { ActivityIcon, ComputerIcon, FileIcon } from "../ui/icons";

/**
 * Rail glyphs for transcript entries. Same family as ui/icons (16-unit
 * grid, 1.5 stroke, round caps); the two missing shapes live here until
 * the shared icon set grows them.
 */

type GlyphProps = Omit<SVGProps<SVGSVGElement>, "children"> & { readonly size?: number };

function Svg({ size = 14, children, ...rest }: GlyphProps & { children: ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5}
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false" {...rest}>
      {children}
    </svg>
  );
}

export const GlobeGlyph = (p: GlyphProps) => (
  <Svg {...p}>
    <circle cx="8" cy="8" r="5.75" vectorEffect="non-scaling-stroke" />
    <path d="M2.5 8h11M8 2.25c1.6 1.6 2.4 3.5 2.4 5.75S9.6 12.15 8 13.75C6.4 12.15 5.6 10.25 5.6 8S6.4 3.85 8 2.25z" vectorEffect="non-scaling-stroke" />
  </Svg>
);

export const TerminalGlyph = (p: GlyphProps) => (
  <Svg {...p}>
    <path d="m3.25 4.75 3 3.25-3 3.25M8.25 11.25h4.5" vectorEffect="non-scaling-stroke" />
  </Svg>
);

export function CapabilityGlyph({ capability, size = 14 }: { readonly capability?: Capability; readonly size?: number }) {
  switch (capability) {
    case "Computer": return <ComputerIcon size={size} />;
    case "Files": return <FileIcon size={size} />;
    case "Web": return <GlobeGlyph size={size} />;
    case "Shell": return <TerminalGlyph size={size} />;
    default: return <ActivityIcon size={size} />;
  }
}
