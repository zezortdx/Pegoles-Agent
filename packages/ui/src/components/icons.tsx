/**
 * Pegoles Flux Glass icon set — one family: 16px grid, 1.5px stroke,
 * round caps and joins, `currentColor`. Decorative by default
 * (aria-hidden); give the owning control an accessible name instead.
 */
import type { ReactNode, SVGProps } from "react";

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, "children"> {
  readonly size?: number;
}

function Icon({ size = 16, children, ...rest }: IconProps & { readonly children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      {children}
    </svg>
  );
}

export function ArrowUpIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 13V3.5M3.75 7.5 8 3.25l4.25 4.25" />
    </Icon>
  );
}

export function PauseIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M5.5 3.5v9M10.5 3.5v9" />
    </Icon>
  );
}

export function PlayIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M5 3.4v9.2a.5.5 0 0 0 .77.42l7.1-4.6a.5.5 0 0 0 0-.84l-7.1-4.6A.5.5 0 0 0 5 3.4Z" />
    </Icon>
  );
}

export function StopIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="4" y="4" width="8" height="8" rx="1.5" />
    </Icon>
  );
}

export function ReturnIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M6 4 3 7l3 3" />
      <path d="M3.5 7h6a3.5 3.5 0 0 1 0 7H8" />
    </Icon>
  );
}

export function PointerIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M3.5 2.75 12.25 7l-3.9 1.15-1.6 3.85L3.5 2.75Z" />
    </Icon>
  );
}

export function CheckIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m3.5 8.25 3 3 6-6.5" />
    </Icon>
  );
}

export function CloseIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m4 4 8 8M12 4l-8 8" />
    </Icon>
  );
}

export function AlertIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="8" cy="8" r="5.75" />
      <path d="M8 5v3.5" />
      <path d="M8 11h.01" strokeWidth={2} />
    </Icon>
  );
}

export function InfoIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="8" cy="8" r="5.75" />
      <path d="M8 7.25V11" />
      <path d="M8 5h.01" strokeWidth={2} />
    </Icon>
  );
}

export function MonitorIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="2" y="2.75" width="12" height="8.5" rx="1.75" />
      <path d="M6 13.75h4M8 11.25v2.5" />
    </Icon>
  );
}

export function WindowIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="2" y="2.75" width="12" height="10.5" rx="1.75" />
      <path d="M2 6h12" />
    </Icon>
  );
}

export function TerminalIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m3.5 5 3 3-3 3M8.5 11.5h4" />
    </Icon>
  );
}

export function TaskIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="2.75" y="2.75" width="10.5" height="10.5" rx="2.5" />
      <path d="m5.5 8.1 1.75 1.75L10.75 6.25" />
    </Icon>
  );
}

/** Rounded octagon: Pegoles' presence shape (not the logo). */
export function PresenceIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M5.9 2.5h4.2a1.5 1.5 0 0 1 1.06.44l1.9 1.9a1.5 1.5 0 0 1 .44 1.06v4.2a1.5 1.5 0 0 1-.44 1.06l-1.9 1.9a1.5 1.5 0 0 1-1.06.44H5.9a1.5 1.5 0 0 1-1.06-.44l-1.9-1.9a1.5 1.5 0 0 1-.44-1.06V5.9a1.5 1.5 0 0 1 .44-1.06l1.9-1.9A1.5 1.5 0 0 1 5.9 2.5Z" />
    </Icon>
  );
}

export function PackageIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m8 1.9 5.25 2.9v6.4L8 14.1l-5.25-2.9V4.8L8 1.9Z" />
      <path d="m2.9 4.9 5.1 2.85 5.1-2.85M8 7.75v6.2" />
    </Icon>
  );
}

export function PowerIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 2.25V7.5" />
      <path d="M4.9 4.4a5 5 0 1 0 6.2 0" />
    </Icon>
  );
}

export function KeyboardIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="1.75" y="4" width="12.5" height="8" rx="1.75" />
      <path d="M4.5 6.75h.01M7 6.75h.01M9.5 6.75h.01M12 6.75h.01M5.5 9.5h5" />
    </Icon>
  );
}
