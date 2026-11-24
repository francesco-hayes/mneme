// The app's icons: small inline SVGs that take the color of the text around
// them. They are decoration; the control that holds one carries the label.

import type { ReactNode } from "react";

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

export function ChevronLeftIcon() {
  return (
    <Icon>
      <path d="M10 3 5 8l5 5" />
    </Icon>
  );
}

export function GearIcon() {
  return (
    <Icon>
      {/* Eight teeth: a dashed ring, each dash a sixteenth of the way round. */}
      <circle
        cx="8"
        cy="8"
        r="6"
        strokeWidth="2"
        strokeDasharray="2.356"
        strokeDashoffset="1.178"
        strokeLinecap="butt"
      />
      <circle cx="8" cy="8" r="4.25" />
      <circle cx="8" cy="8" r="1.5" strokeWidth="1.25" />
    </Icon>
  );
}

/** Two chevrons, up and down: a control that opens a list of choices. */
export function ChevronUpDownIcon() {
  return (
    <Icon>
      <path d="m5 6 3-3 3 3M5 10l3 3 3-3" />
    </Icon>
  );
}

export function TrashIcon() {
  return (
    <Icon>
      <path d="M2.75 4.25h10.5M6.25 4.25v-1.5h3.5v1.5M4 4.25l.6 8.5a1 1 0 0 0 1 .95h4.8a1 1 0 0 0 1-.95l.6-8.5M6.75 7v4M9.25 7v4" />
    </Icon>
  );
}

/** Three dots in a row: a menu of more actions. */
export function MoreIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="3.5" cy="8" r="1.25" />
      <circle cx="8" cy="8" r="1.25" />
      <circle cx="12.5" cy="8" r="1.25" />
    </svg>
  );
}

export function PlusIcon() {
  return (
    <Icon>
      <path d="M8 3v10M3 8h10" />
    </Icon>
  );
}

/** A filled square, as on a recorder's stop key. */
export function StopIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="4" y="4" width="8" height="8" rx="1.5" />
    </svg>
  );
}

export function WarningIcon() {
  return (
    <Icon>
      <circle cx="8" cy="8" r="6.25" />
      <path d="M8 4.75v3.75M8 11.1v.15" />
    </Icon>
  );
}

/** Lines of speech from two sides: a transcript. */
export function TranscriptIcon() {
  return (
    <Icon>
      <path d="M2.5 4h7M6.5 8h7M2.5 12h7" />
    </Icon>
  );
}

/** A tick: the chosen item in a list, or something that is in place. */
export function CheckIcon() {
  return (
    <Icon>
      <path d="m3.5 8.5 3 3 6-7" />
    </Icon>
  );
}
