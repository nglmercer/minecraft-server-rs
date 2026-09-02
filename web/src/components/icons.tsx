import type { JSX } from "preact";

/**
 * Inline SVG icons.
 *
 * Inline rather than an icon font or a sprite sheet: the panel is embedded in
 * the binary and served under a strict CSP, so anything fetched from elsewhere
 * would not load. They inherit `currentColor` and size from the `size` prop.
 */
type IconProps = Omit<JSX.SVGAttributes<SVGSVGElement>, "size"> & { size?: number };

function Icon({ size = 16, children, ...props }: IconProps & { children: JSX.Element }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {children}
    </svg>
  );
}

export const Play = (p: IconProps) => (
  <Icon {...p}>
    <polygon points="6 3 20 12 6 21 6 3" fill="currentColor" stroke="none" />
  </Icon>
);

export const Stop = (p: IconProps) => (
  <Icon {...p}>
    <>
      <circle cx="12" cy="12" r="9" />
      <rect x="9" y="9" width="6" height="6" rx="1" fill="currentColor" stroke="none" />
    </>
  </Icon>
);

export const Restart = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M21 12a9 9 0 1 1-2.64-6.36" />
      <polyline points="21 3 21 9 15 9" />
    </>
  </Icon>
);

export const Power = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M12 3v9" />
      <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
    </>
  </Icon>
);

export const Dots = (p: IconProps) => (
  <Icon {...p}>
    <>
      <circle cx="12" cy="5" r="1.5" fill="currentColor" stroke="none" />
      <circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none" />
      <circle cx="12" cy="19" r="1.5" fill="currentColor" stroke="none" />
    </>
  </Icon>
);

export const Cpu = (p: IconProps) => (
  <Icon {...p}>
    <>
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <rect x="10" y="10" width="4" height="4" rx="1" />
      <path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3" />
    </>
  </Icon>
);

export const Memory = (p: IconProps) => (
  <Icon {...p}>
    <>
      <ellipse cx="12" cy="6" rx="8" ry="3" />
      <path d="M4 6v6c0 1.66 3.58 3 8 3s8-1.34 8-3V6" />
      <path d="M4 12v6c0 1.66 3.58 3 8 3s8-1.34 8-3v-6" />
    </>
  </Icon>
);

export const Folder = (p: IconProps) => (
  <Icon {...p}>
    <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
  </Icon>
);

export const FolderOpen = (p: IconProps) => (
  <Icon {...p}>
    <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v1H3zM3 10h18l-2 8a2 2 0 0 1-2 1H5a2 2 0 0 1-2-2z" />
  </Icon>
);

export const File = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
      <polyline points="14 3 14 8 19 8" />
    </>
  </Icon>
);

export const Clock = (p: IconProps) => (
  <Icon {...p}>
    <>
      <circle cx="12" cy="12" r="9" />
      <polyline points="12 7 12 12 15 14" />
    </>
  </Icon>
);

export const Link = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.5 1.5" />
      <path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.5-1.5" />
    </>
  </Icon>
);

export const Gamepad = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M7 8h10a5 5 0 0 1 5 5v1a4 4 0 0 1-7.2 2.4L14 15h-4l-.8 1.4A4 4 0 0 1 2 14v-1a5 5 0 0 1 5-5z" />
      <path d="M7 11v2M6 12h2M16 12h.01M18 14h.01" />
    </>
  </Icon>
);

export const Tag = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M3 12V5a2 2 0 0 1 2-2h7l9 9-9 9z" />
      <circle cx="7.5" cy="7.5" r="1.2" fill="currentColor" stroke="none" />
    </>
  </Icon>
);

export const Expand = (p: IconProps) => (
  <Icon {...p}>
    <path d="M8 3H5a2 2 0 0 0-2 2v3M16 3h3a2 2 0 0 1 2 2v3M8 21H5a2 2 0 0 1-2-2v-3M16 21h3a2 2 0 0 0 2-2v-3" />
  </Icon>
);

export const Collapse = (p: IconProps) => (
  <Icon {...p}>
    <path d="M3 8h3a2 2 0 0 0 2-2V3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M21 16h-3a2 2 0 0 0-2 2v3" />
  </Icon>
);

export const ArrowDown = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M12 5v14" />
      <polyline points="6 13 12 19 18 13" />
    </>
  </Icon>
);

export const ArrowLeft = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M19 12H5" />
      <polyline points="11 6 5 12 11 18" />
    </>
  </Icon>
);

export const Upload = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M12 16V4" />
      <polyline points="7 9 12 4 17 9" />
      <path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" />
    </>
  </Icon>
);

export const Download = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M12 4v12" />
      <polyline points="7 11 12 16 17 11" />
      <path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" />
    </>
  </Icon>
);

export const Copy = (p: IconProps) => (
  <Icon {...p}>
    <>
      <rect x="8" y="8" width="12" height="12" rx="2" />
      <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
    </>
  </Icon>
);

export const Trash = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M4 7h16" />
      <path d="M10 11v6M14 11v6" />
      <path d="M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12" />
      <path d="M9 7V5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2" />
    </>
  </Icon>
);

export const Pencil = (p: IconProps) => (
  <Icon {...p}>
    <path d="M4 20h4l10-10a2.83 2.83 0 0 0-4-4L4 16z" />
  </Icon>
);

export const Search = (p: IconProps) => (
  <Icon {...p}>
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="M21 21l-4.3-4.3" />
    </>
  </Icon>
);

export const Plus = (p: IconProps) => (
  <Icon {...p}>
    <path d="M12 5v14M5 12h14" />
  </Icon>
);

export const X = (p: IconProps) => (
  <Icon {...p}>
    <path d="M18 6 6 18M6 6l12 12" />
  </Icon>
);

export const Terminal = (p: IconProps) => (
  <Icon {...p}>
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <polyline points="7 9 10 12 7 15" />
      <path d="M13 15h4" />
    </>
  </Icon>
);

export const Package = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z" />
      <path d="M4 7.5l8 4.5 8-4.5M12 12v9" />
    </>
  </Icon>
);

export const Archive = (p: IconProps) => (
  <Icon {...p}>
    <>
      <rect x="3" y="4" width="18" height="4" rx="1" />
      <path d="M5 8v11a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8" />
      <path d="M10 12h4" />
    </>
  </Icon>
);

export const Settings = (p: IconProps) => (
  <Icon {...p}>
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </>
  </Icon>
);

export const Users = (p: IconProps) => (
  <Icon {...p}>
    <>
      <circle cx="9" cy="8" r="3.5" />
      <path d="M2.5 20a6.5 6.5 0 0 1 13 0" />
      <path d="M16 5.2a3.5 3.5 0 0 1 0 6.6M18 14.5a6.5 6.5 0 0 1 3.5 5.5" />
    </>
  </Icon>
);

export const LogOut = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M9 21H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3" />
      <polyline points="16 17 21 12 16 7" />
      <path d="M21 12H9" />
    </>
  </Icon>
);

export const Globe = (p: IconProps) => (
  <Icon {...p}>
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18M12 3a15 15 0 0 1 0 18 15 15 0 0 1 0-18z" />
    </>
  </Icon>
);

export const Refresh = (p: IconProps) => (
  <Icon {...p}>
    <>
      <polyline points="21 4 21 10 15 10" />
      <polyline points="3 20 3 14 9 14" />
      <path d="M20 9a8 8 0 0 0-14-3L3 9M4 15a8 8 0 0 0 14 3l3-3" />
    </>
  </Icon>
);

export const Warning = (p: IconProps) => (
  <Icon {...p}>
    <>
      <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" />
      <path d="M12 9v4M12 17h.01" />
    </>
  </Icon>
);

export const Info = (p: IconProps) => (
  <Icon {...p}>
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 16v-4M12 8h.01" />
    </>
  </Icon>
);
