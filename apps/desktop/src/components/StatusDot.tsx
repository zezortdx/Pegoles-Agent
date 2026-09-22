export function StatusDot({ on, tone = "auto" }: { on: boolean; tone?: "auto" | "warn" }) {
  const color = !on ? "#6b6b64" : tone === "warn" ? "#c2a878" : "#8fa38f";
  return (
    <span
      aria-hidden
      style={{
        display: "inline-block",
        width: 8,
        height: 8,
        borderRadius: 999,
        background: color,
        marginRight: 8,
        verticalAlign: "baseline",
      }}
    />
  );
}
