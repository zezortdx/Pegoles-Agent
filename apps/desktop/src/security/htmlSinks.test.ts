import { describe, expect, it } from "vitest";

/**
 * The frontend never renders model, guest or error text as HTML: it only
 * ever reaches the page as React text. This scan keeps every HTML or script
 * sink out of the app and the shared UI package, so no future component can
 * start doing so quietly. Tests are excluded (they build hostile markup on
 * purpose).
 */
const sources = import.meta.glob<string>(
  [
    "../**/*.{ts,tsx}",
    "!../**/*.test.{ts,tsx}",
    "../../../../packages/ui/src/**/*.{ts,tsx}",
    "!../../../../packages/ui/src/**/*.test.{ts,tsx}",
  ],
  { query: "?raw", import: "default", eager: true },
);

/** Each sink, and why it is banned. */
const SINKS: readonly (readonly [RegExp, string])[] = [
  [/dangerouslySetInnerHTML/, "renders a string as HTML"],
  [/\.(inner|outer)HTML\b/, "parses a string as HTML"],
  [/insertAdjacentHTML/, "parses a string as HTML"],
  [/createContextualFragment/, "parses a string as HTML"],
  [/\bDOMParser\b/, "parses a string as HTML"],
  [/document\.write/, "writes a string as HTML"],
  [/\bsrcDoc\b|\bsrcdoc\b/, "loads a string as a document"],
  [/\bnew Function\s*\(|\beval\s*\(/, "runs a string as code"],
  [/javascript:/i, "a script URL"],
  [/\bhref=\{|xlinkHref|formAction=/, "a link or form target built from data (use a checked, fixed URL)"],
];

describe("HTML and script sinks", () => {
  it("scans the real sources", () => {
    const files = Object.keys(sources);
    expect(files.some((file) => file.endsWith("/App.tsx"))).toBe(true);
    expect(files.some((file) => file.includes("packages/ui/src/"))).toBe(true);
    expect(files.some((file) => file.includes(".test."))).toBe(false);
  });

  for (const [sink, why] of SINKS) {
    it(`no component uses ${sink.source} (${why})`, () => {
      const offenders = Object.entries(sources).filter(([, source]) => sink.test(source)).map(([file]) => file);
      expect(offenders).toEqual([]);
    });
  }
});
