import type { SVGProps } from "react";

/** Ícones próprios (traços simples, 16×16). Nenhum ativo de terceiros. */
const PATHS = {
  play: "M5 3.5v9l7-4.5z",
  pause: "M4.5 3.5h2.5v9H4.5zM9 3.5h2.5v9H9z",
  stepBack: "M3.5 3.5v9M12.5 3.5L6 8l6.5 4.5z",
  stepForward: "M12.5 3.5v9M3.5 3.5L10 8l-6.5 4.5z",
  undo: "M6.5 4L3 7.5 6.5 11M3.5 7.5H10a3 3 0 010 6H8",
  redo: "M9.5 4L13 7.5 9.5 11M12.5 7.5H6a3 3 0 000 6h2",
  scissors:
    "M4 4.5a1.5 1.5 0 100 3 1.5 1.5 0 000-3zM4 8.5a1.5 1.5 0 100 3 1.5 1.5 0 000-3zM5.2 5.6L13 12M5.2 10.4L13 4",
  trash: "M3.5 4.5h9M6.5 4.5v-1h3v1M4.5 4.5l.5 8h6l.5-8M7 6.5v4M9 6.5v4",
  magnet: "M4 3v5a4 4 0 008 0V3M4 5.5h2.5M9.5 5.5H12",
  link: "M6.5 9.5l3-3M5 8l-1.2 1.2a2.1 2.1 0 003 3L8 11M11 8l1.2-1.2a2.1 2.1 0 00-3-3L8 5",
  plus: "M8 3.5v9M3.5 8h9",
  close: "M4.5 4.5l7 7M11.5 4.5l-7 7",
  folder: "M2.5 4.5h4l1.5 1.5h5.5v6.5h-11z",
  film: "M3 3.5h10v9H3zM3 6h10M3 10h10M5.5 3.5v9M10.5 3.5v9",
  music:
    "M6.5 11.5V4.5l6-1.5v7M4.5 13a1.5 1.5 0 100-3 1.5 1.5 0 000 3zM10.5 11.5a1.5 1.5 0 100-3 1.5 1.5 0 000 3z",
  image: "M2.5 3.5h11v9h-11zM2.5 11l3.5-3.5 3 3 2-2 2.5 2.5M10.5 6a.9.9 0 100-1.8.9.9 0 000 1.8z",
  text: "M3.5 4.5v-1h9v1M8 3.5v9M6 12.5h4",
  caption: "M2.5 4h11v8h-11zM4.5 7.5h3M9 7.5h2.5M4.5 9.5h2M8 9.5h3.5",
  transition: "M2.5 4h5v8h-5zM8.5 4h5v8h-5zM6 8h4M8.5 6.5L10 8l-1.5 1.5",
  eye: "M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8zM8 9.8a1.8 1.8 0 100-3.6 1.8 1.8 0 000 3.6z",
  eyeOff:
    "M2 2l12 12M6.4 4a6 6 0 011.6-.5c4 0 6.5 4.5 6.5 4.5a11 11 0 01-2 2.5M4.2 5.4A11 11 0 001.5 8S4 12.5 8 12.5a6 6 0 002.2-.4",
  lock: "M4.5 7.5h7v5h-7zM6 7.5v-2a2 2 0 014 0v2",
  unlock: "M4.5 7.5h7v5h-7zM6 7.5v-2a2 2 0 013.7-1",
  volume: "M3 6.5h2.5L9 3.5v9L5.5 9.5H3zM11 6a3 3 0 010 4M12.5 4.5a5 5 0 010 7",
  mute: "M3 6.5h2.5L9 3.5v9L5.5 9.5H3zM11 6l3 4M14 6l-3 4",
  solo: "M8 3.5a2 2 0 100 4 2 2 0 000-4zM4 12.5c.5-2.2 2-3.5 4-3.5s3.5 1.3 4 3.5",
  zoomIn: "M7 3.5a3.5 3.5 0 100 7 3.5 3.5 0 000-7zM9.6 9.6L13 13M5.5 7h3M7 5.5v3",
  zoomOut: "M7 3.5a3.5 3.5 0 100 7 3.5 3.5 0 000-7zM9.6 9.6L13 13M5.5 7h3",
  fit: "M3 6V3h3M10 3h3v3M13 10v3h-3M6 13H3v-3",
  export: "M8 10V3M5 5.5L8 2.5l3 3M3.5 9v4h9V9",
  import: "M8 3v7M5 7.5L8 10.5l3-3M3.5 9v4h9V9",
  settings:
    "M8 5.5a2.5 2.5 0 100 5 2.5 2.5 0 000-5zM8 2v1.5M8 12.5V14M2 8h1.5M12.5 8H14M3.8 3.8l1 1M11.2 11.2l1 1M3.8 12.2l1-1M11.2 4.8l1-1",
  history: "M3 8a5 5 0 105-5H5.5M5.5 1.5V4h2.5M8 5.5V8l2 1.2",
  warning: "M8 2.5l6 10.5H2zM8 6.5v3M8 11v.5",
  chevronRight: "M6 4l4 4-4 4",
  chevronDown: "M4 6l4 4 4-4",
  duplicate: "M5.5 5.5h7v7h-7zM3.5 10.5v-7h7",
  group: "M2.5 5.5h4v4h-4zM9.5 6.5h4v4h-4zM6.5 7.5h3",
  nested: "M2.5 3.5h8v6h-8zM5.5 6.5h8v6h-8z",
  keyframe: "M8 2.5L13.5 8 8 13.5 2.5 8z",
  grid: "M3 3h4v4H3zM9 3h4v4H9zM3 9h4v4H3zM9 9h4v4H9z",
  search: "M7 3.5a3.5 3.5 0 100 7 3.5 3.5 0 000-7zM9.6 9.6L13 13",
  fullscreen: "M3 6V3h3M10 3h3v3M13 10v3h-3M6 13H3v-3",
  fullscreenExit: "M6 3v3H3M13 6h-3V3M10 13v-3h3M3 10h3v3",
} as const;

export type IconName = keyof typeof PATHS;

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, "name"> {
  name: IconName;
  size?: number;
}

export function Icon({ name, size = 16, ...rest }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
