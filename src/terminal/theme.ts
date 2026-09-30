import type { ITheme } from "@xterm/xterm";

// Backgrounds match the app palette in src/app/app.css.
const dark: ITheme = {
  background: "#141414",
  foreground: "#e8e8e8",
  cursor: "#e0607e",
  cursorAccent: "#141414",
  selectionBackground: "#3a3080",
  black: "#202020",
  red: "#ff6b6b",
  green: "#5fbf6a",
  yellow: "#d9a441",
  blue: "#7c6cff",
  magenta: "#e0607e",
  cyan: "#4fc3cf",
  white: "#bdbdbd",
  brightBlack: "#6e6e6e",
  brightRed: "#ff8f8f",
  brightGreen: "#7fd48a",
  brightYellow: "#ecc062",
  brightBlue: "#9d91ff",
  brightMagenta: "#f08aa3",
  brightCyan: "#72d9e3",
  brightWhite: "#f2f2f2",
};

const light: ITheme = {
  background: "#f6f6f6",
  foreground: "#1a1a1a",
  cursor: "#c2415f",
  cursorAccent: "#f6f6f6",
  selectionBackground: "#d4ceff",
  black: "#1f1f1f",
  red: "#cf222e",
  green: "#116329",
  yellow: "#9a6700",
  blue: "#3d1fff",
  magenta: "#c2415f",
  cyan: "#1b7c83",
  white: "#6e6e6e",
  brightBlack: "#575757",
  brightRed: "#a40e26",
  brightGreen: "#1a7f37",
  brightYellow: "#633c01",
  brightBlue: "#5a42ff",
  brightMagenta: "#d85a78",
  brightCyan: "#3192aa",
  brightWhite: "#8c8c8c",
};

export function terminalTheme(prefersDark: boolean): ITheme {
  return prefersDark ? dark : light;
}
