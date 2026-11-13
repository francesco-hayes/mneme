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
