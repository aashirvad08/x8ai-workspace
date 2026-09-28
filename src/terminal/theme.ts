import type { ITheme } from "@xterm/xterm";

// Backgrounds match the app palette in src/app/app.css.
const dark: ITheme = {
  background: "#0f1115",
  foreground: "#d7dce3",
  cursor: "#5aa9ff",
  cursorAccent: "#0f1115",
  selectionBackground: "#2b3b55",
  black: "#1c2027",
  red: "#f47067",
  green: "#57ab5a",
  yellow: "#c69026",
  blue: "#539bf5",
  magenta: "#b083f0",
  cyan: "#39c5cf",
  white: "#adbac7",
  brightBlack: "#636e7b",
  brightRed: "#ff938a",
  brightGreen: "#6bc46d",
  brightYellow: "#daaa3f",
  brightBlue: "#6cb6ff",
  brightMagenta: "#dcbdfb",
  brightCyan: "#56d4dd",
  brightWhite: "#e6edf3",
};

const light: ITheme = {
  background: "#f7f8fa",
  foreground: "#1f2328",
  cursor: "#0969da",
  cursorAccent: "#f7f8fa",
  selectionBackground: "#b6d6fb",
  black: "#24292f",
  red: "#cf222e",
  green: "#116329",
  yellow: "#9a6700",
  blue: "#0969da",
  magenta: "#8250df",
  cyan: "#1b7c83",
  white: "#6e7781",
  brightBlack: "#57606a",
  brightRed: "#a40e26",
  brightGreen: "#1a7f37",
  brightYellow: "#633c01",
  brightBlue: "#218bff",
  brightMagenta: "#a475f9",
  brightCyan: "#3192aa",
  brightWhite: "#8c959f",
};

export function terminalTheme(prefersDark: boolean): ITheme {
  return prefersDark ? dark : light;
}
