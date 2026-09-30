import type { ReactNode, SVGProps } from "react";

/**
 * One icon family: 16-unit grid, 1.5 stroke at every size, round caps,
 * currentColor. Sizes 14 / 16 / 20. Decorative unless given a title.
 */
interface IconProps extends Omit<SVGProps<SVGSVGElement>, "children"> {
  readonly size?: number;
}

function Icon({ size = 16, children, ...rest }: IconProps & { children: ReactNode }) {
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

export const PlusIcon = (p: IconProps) => <Icon {...p}><path d="M8 3.25v9.5M3.25 8h9.5" vectorEffect="non-scaling-stroke" /></Icon>;
export const ArrowUpIcon = (p: IconProps) => <Icon {...p}><path d="M8 12.75V3.5M4 7.25 8 3.25l4 4" vectorEffect="non-scaling-stroke" /></Icon>;
export const CloseIcon = (p: IconProps) => <Icon {...p}><path d="m4.25 4.25 7.5 7.5m0-7.5-7.5 7.5" vectorEffect="non-scaling-stroke" /></Icon>;
export const ChevronRightIcon = (p: IconProps) => <Icon {...p}><path d="m6.25 3.75 4.25 4.25-4.25 4.25" vectorEffect="non-scaling-stroke" /></Icon>;
export const CheckIcon = (p: IconProps) => <Icon {...p}><path d="m3.5 8.25 3 3 6-6.5" vectorEffect="non-scaling-stroke" /></Icon>;
export const AlertIcon = (p: IconProps) => <Icon {...p}><path d="M8 4.5v4.25" vectorEffect="non-scaling-stroke" /><circle cx="8" cy="11.25" r="0.4" fill="currentColor" /></Icon>;
export const MoreIcon = (p: IconProps) => (
  <Icon {...p}>
    <circle cx="3.75" cy="8" r="0.6" fill="currentColor" />
    <circle cx="8" cy="8" r="0.6" fill="currentColor" />
    <circle cx="12.25" cy="8" r="0.6" fill="currentColor" />
  </Icon>
);
export const SidebarIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="2" y="2.75" width="12" height="10.5" rx="2.25" vectorEffect="non-scaling-stroke" />
    <path d="M6.25 2.75v10.5" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const ComputerIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="1.75" y="2.5" width="12.5" height="8.5" rx="1.75" vectorEffect="non-scaling-stroke" />
    <path d="M6 13.5h4M8 11v2.5" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const ActivityIcon = (p: IconProps) => <Icon {...p}><path d="M1.75 8h2.5l1.75-4.25 4 8.5L11.75 8h2.5" vectorEffect="non-scaling-stroke" /></Icon>;
export const SettingsIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M2.5 4.75h6.25M12 4.75h1.5M2.5 11.25H4M7.25 11.25h6.25" vectorEffect="non-scaling-stroke" />
    <circle cx="10.4" cy="4.75" r="1.6" vectorEffect="non-scaling-stroke" />
    <circle cx="5.6" cy="11.25" r="1.6" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const ExpandIcon = (p: IconProps) => <Icon {...p}><path d="M9.75 2.75h3.5v3.5M6.25 13.25h-3.5v-3.5M13.25 2.75 9 7M2.75 13.25 7 9" vectorEffect="non-scaling-stroke" /></Icon>;
export const CollapseIcon = (p: IconProps) => <Icon {...p}><path d="M13.25 6.25h-3.5v-3.5M2.75 9.75h3.5v3.5M9.75 6.25 14 2M6.25 9.75 2 14" vectorEffect="non-scaling-stroke" /></Icon>;
export const FileIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M9.25 1.75H4.5A1.25 1.25 0 0 0 3.25 3v10A1.25 1.25 0 0 0 4.5 14.25h7A1.25 1.25 0 0 0 12.75 13V5.25z" vectorEffect="non-scaling-stroke" />
    <path d="M9.25 1.75v3.5h3.5" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const SearchIcon = (p: IconProps) => <Icon {...p}><circle cx="7" cy="7" r="4.25" vectorEffect="non-scaling-stroke" /><path d="m10.25 10.25 3.5 3.5" vectorEffect="non-scaling-stroke" /></Icon>;
export const ShieldIcon = (p: IconProps) => <Icon {...p}><path d="M8 1.75 2.75 3.75v4c0 3.1 2.2 5.4 5.25 6.5 3.05-1.1 5.25-3.4 5.25-6.5v-4z" vectorEffect="non-scaling-stroke" /></Icon>;
export const PowerIcon = (p: IconProps) => <Icon {...p}><path d="M8 2.25v5.25" vectorEffect="non-scaling-stroke" /><path d="M4.6 4.2a5 5 0 1 0 6.8 0" vectorEffect="non-scaling-stroke" /></Icon>;
export const ComposeIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M7.25 2.75H4.5A1.75 1.75 0 0 0 2.75 4.5v7A1.75 1.75 0 0 0 4.5 13.25h7a1.75 1.75 0 0 0 1.75-1.75V8.75" vectorEffect="non-scaling-stroke" />
    <path d="M11.9 2.35a1.2 1.2 0 0 1 1.7 1.7L8.5 9.15l-2.25.6.6-2.25z" vectorEffect="non-scaling-stroke" />
  </Icon>
);
/** The right-hand inspector (Pegoles Computer) shown or hidden. */
export const PanelRightIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="2" y="2.75" width="12" height="10.5" rx="2.25" vectorEffect="non-scaling-stroke" />
    <path d="M9.75 2.75v10.5" vectorEffect="non-scaling-stroke" />
  </Icon>
);
/** Give the computer more room (Focus). */
export const FocusIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="2" y="2.75" width="12" height="10.5" rx="2.25" vectorEffect="non-scaling-stroke" />
    <path d="M6 2.75v10.5M8.25 8h3M9.75 6.5 11.25 8l-1.5 1.5" vectorEffect="non-scaling-stroke" />
  </Icon>
);
/** Back to a narrower computer. */
export const UnfocusIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="2" y="2.75" width="12" height="10.5" rx="2.25" vectorEffect="non-scaling-stroke" />
    <path d="M6 2.75v10.5M11.25 8h-3M9.75 6.5 8.25 8l1.5 1.5" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const FullscreenIcon = (p: IconProps) => <Icon {...p}><path d="M2.75 6V2.75H6M10 2.75h3.25V6M13.25 10v3.25H10M6 13.25H2.75V10" vectorEffect="non-scaling-stroke" /></Icon>;
export const ExitFullscreenIcon = (p: IconProps) => <Icon {...p}><path d="M6 2.75V6H2.75M13.25 6H10V2.75M10 13.25V10h3.25M2.75 10H6v3.25" vectorEffect="non-scaling-stroke" /></Icon>;
export const RefreshIcon = (p: IconProps) => <Icon {...p}><path d="M13 3.25v3h-3M12.6 6.25A5 5 0 1 0 13 9.5" vectorEffect="non-scaling-stroke" /></Icon>;
export const StopIcon = (p: IconProps) => <Icon {...p}><rect x="4.25" y="4.25" width="7.5" height="7.5" rx="1.5" fill="currentColor" stroke="none" /></Icon>;
export const PauseIcon = (p: IconProps) => <Icon {...p}><path d="M5.75 4v8M10.25 4v8" vectorEffect="non-scaling-stroke" /></Icon>;
export const ArrowLeftIcon = (p: IconProps) => <Icon {...p}><path d="M12.75 8H3.5M7.25 4 3.25 8l4 4" vectorEffect="non-scaling-stroke" /></Icon>;
export const ChevronDownIcon = (p: IconProps) => <Icon {...p}><path d="m3.75 6.25 4.25 4.25 4.25-4.25" vectorEffect="non-scaling-stroke" /></Icon>;
export const GlobeIcon = (p: IconProps) => (
  <Icon {...p}>
    <circle cx="8" cy="8" r="5.75" vectorEffect="non-scaling-stroke" />
    <path d="M2.5 8h11M8 2.25c1.6 1.6 2.4 3.5 2.4 5.75S9.6 12.15 8 13.75C6.4 12.15 5.6 10.25 5.6 8S6.4 3.85 8 2.25z" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const TerminalIcon = (p: IconProps) => <Icon {...p}><path d="m3.25 4.75 3 3.25-3 3.25M8.25 11.25h4.5" vectorEffect="non-scaling-stroke" /></Icon>;
export const FolderIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.6l1.4 1.5h5a1.25 1.25 0 0 1 1.25 1.25v5.5a1.25 1.25 0 0 1-1.25 1.25h-9A1.25 1.25 0 0 1 2.25 11.5z" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const HandIcon = (p: IconProps) => (
  <Icon {...p}>
    <path d="M5.25 8.5V4a.9.9 0 0 1 1.8 0v3.5M7.05 7V3.1a.9.9 0 0 1 1.8 0V7M8.85 7V3.9a.9.9 0 0 1 1.8 0v4.35M10.65 7.25a.9.9 0 0 1 1.8 0v1.9c0 2.7-1.7 4.6-4.2 4.6-1.6 0-2.6-.7-3.5-2l-1.5-2.3a.95.95 0 0 1 1.5-1.15l.55.65" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const AlertCircleIcon = (p: IconProps) => (
  <Icon {...p}>
    <circle cx="8" cy="8" r="5.75" vectorEffect="non-scaling-stroke" />
    <path d="M8 5.25v3.25" vectorEffect="non-scaling-stroke" />
    <circle cx="8" cy="10.75" r="0.4" fill="currentColor" />
  </Icon>
);
/** The model: a chip. */
export const ModelIcon = (p: IconProps) => (
  <Icon {...p}>
    <rect x="4" y="4" width="8" height="8" rx="1.75" vectorEffect="non-scaling-stroke" />
    <path d="M6.5 2v2M9.5 2v2M6.5 12v2M9.5 12v2M2 6.5h2M2 9.5h2M12 6.5h2M12 9.5h2" vectorEffect="non-scaling-stroke" />
  </Icon>
);
export const PersonIcon = (p: IconProps) => (
  <Icon {...p}>
    <circle cx="8" cy="5.25" r="2.5" vectorEffect="non-scaling-stroke" />
    <path d="M3 13.25c.6-2.4 2.6-3.75 5-3.75s4.4 1.35 5 3.75" vectorEffect="non-scaling-stroke" />
  </Icon>
);
