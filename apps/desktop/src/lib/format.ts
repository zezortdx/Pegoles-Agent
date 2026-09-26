/** Host platform label for people, not identifiers ("macos" → "macOS"). */
const PLATFORM_LABEL: Record<string, string> = { macos: "macOS", windows: "Windows", linux: "Linux" };
const ARCH_LABEL: Record<string, string> = { arm64: "Apple silicon", aarch64: "ARM64", x86_64: "x64", amd64: "x64" };

export function hostLabel(platform: string, architecture: string): string {
  const os = PLATFORM_LABEL[platform] ?? platform;
  const arch = platform === "macos" && architecture === "arm64" ? ARCH_LABEL.arm64 : ARCH_LABEL[architecture] ?? architecture;
  return `${os} · ${arch}`;
}

/** "not_configured" → "Not configured". */
export function sentenceCase(wire: string): string {
  const words = wire.replaceAll("_", " ").trim();
  return words ? words[0].toUpperCase() + words.slice(1) : words;
}

/** The modifier people press for app shortcuts on this machine. */
export function shortcutModifier(): string {
  if (typeof navigator === "undefined") return "Ctrl";
  const platform = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform ?? navigator.platform ?? "";
  return /mac|iphone|ipad/i.test(platform) ? "⌘" : "Ctrl";
}

/** Compact elapsed time for live work: "34s", "12m", "2h 5m". */
export function formatElapsed(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours}h ${rest}m` : `${hours}h`;
}

/** "claude-opus-5-5" → "Claude Opus 5.5"; unknown ids stay as they are. */
export function modelLabel(id: string): string {
  const match = /^claude-([a-z]+)-(\d+(?:-\d+)*)$/.exec(id);
  if (!match) return id;
  const family = match[1].charAt(0).toUpperCase() + match[1].slice(1);
  return `Claude ${family} ${match[2].replaceAll("-", ".")}`;
}

const EFFORT_LABEL: Record<string, string> = { low: "Low", medium: "Medium", high: "High", xhigh: "Extra high", max: "Max" };

/** Reasoning effort for people ("xhigh" → "Extra high"). */
export function effortLabel(effort: string): string {
  return EFFORT_LABEL[effort] ?? sentenceCase(effort);
}
