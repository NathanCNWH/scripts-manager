/** Shared inline SVG icons (Lucide-style, 24x24 viewBox, stroke-based). */

/** Base wrapper providing consistent sizing and stroke styling. */
function Icon({ size = 16, filled = false, children }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke={filled ? "none" : "currentColor"}
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

/** Triangle play symbol. */
export function PlayIcon({ size }) {
  return (
    <Icon size={size} filled>
      <polygon points="6 3 20 12 6 21 6 3" />
    </Icon>
  );
}

/** Two-bar pause symbol. */
export function PauseIcon({ size }) {
  return (
    <Icon size={size} filled>
      <rect x="5" y="4" width="5" height="16" rx="1" />
      <rect x="14" y="4" width="5" height="16" rx="1" />
    </Icon>
  );
}

/** Square stop symbol. */
export function StopIcon({ size }) {
  return (
    <Icon size={size} filled>
      <rect x="5" y="5" width="14" height="14" rx="2" />
    </Icon>
  );
}

/** Plus symbol for add/import actions. */
export function PlusIcon({ size }) {
  return (
    <Icon size={size}>
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </Icon>
  );
}

/** Pencil symbol for editing a registered entry. */
export function EditIcon({ size }) {
  return (
    <Icon size={size}>
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L9 17l-4 1 1-4Z" />
    </Icon>
  );
}

/** Cross symbol for close/dismiss actions. */
export function XIcon({ size }) {
  return (
    <Icon size={size}>
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </Icon>
  );
}

/** Terminal prompt symbol. */
export function TerminalIcon({ size }) {
  return (
    <Icon size={size}>
      <polyline points="4 17 10 11 4 5" />
      <line x1="12" y1="19" x2="20" y2="19" />
    </Icon>
  );
}

/** Sun symbol for switching to the light theme. */
export function SunIcon({ size }) {
  return (
    <Icon size={size}>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2" />
      <path d="M12 20v2" />
      <path d="m4.93 4.93 1.41 1.41" />
      <path d="m17.66 17.66 1.41 1.41" />
      <path d="M2 12h2" />
      <path d="M20 12h2" />
      <path d="m6.34 17.66-1.41 1.41" />
      <path d="m19.07 4.93-1.41 1.41" />
    </Icon>
  );
}

/** Moon symbol for switching to the dark theme. */
export function MoonIcon({ size }) {
  return (
    <Icon size={size}>
      <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
    </Icon>
  );
}

/** Check mark for toggleable menu items. */
export function CheckIcon({ size }) {
  return (
    <Icon size={size}>
      <polyline points="20 6 9 17 4 12" />
    </Icon>
  );
}

/** Trash symbol for clearing the terminal history. */
export function TrashIcon({ size }) {
  return (
    <Icon size={size}>
      <path d="M3 6h18" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
      <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </Icon>
  );
}

/** Code file symbol for empty states. */
export function FileCodeIcon({ size }) {
  return (
    <Icon size={size}>
      <path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z" />
      <polyline points="14 2 14 8 20 8" />
      <path d="m10 13-2 2 2 2" />
      <path d="m14 17 2-2-2-2" />
    </Icon>
  );
}
