import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";

import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import { type SessionEnding, type SessionNative, TerminalSession } from "./session";
import type { TerminalReader } from "./terminals";
import { terminalTheme } from "./theme";

/** The app's own terminal font. */
export const DEFAULT_FONT = '"SF Mono", Menlo, Monaco, monospace';

interface Props {
  /** Starts the session (a shell, or an agent) and drives it. Must be stable. */
  native: SessionNative;
  /** An agent's name, for messages; the user's shell if unset. */
  program?: string | undefined;
  /** Runs once: Enter does not start it again when it ends (an add-on install). */
  once?: boolean;
  /** The font family; changing it re-renders the terminal in place. */
  fontFamily?: string;
  /** Hidden views (in other tabs) keep running. */
  visible: boolean;
  /** The pane that should have keyboard focus in its tab. */
  focused: boolean;
  /** Focuses the terminal when this changes while it is visible and focused. */
  focusRequest: number;
  onStart?: (info: TerminalInfo) => void;
  onEnd?: (ending: SessionEnding) => void;
  /** Lets the app read the terminal and paste into it; `null` when it closes. */
  onReader?: (reader: TerminalReader | null) => void;
}

/**
 * Renders one terminal session with xterm.js. This component only wires the
 * emulator to the DOM; session behaviour lives in `TerminalSession`.
 */
export function TerminalView({
  native,
  program,
  once = false,
  fontFamily = DEFAULT_FONT,
  visible,
  focused,
  focusRequest,
  onStart,
  onEnd,
  onReader,
}: Props) {
  const container = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  // Read when the terminal is made; later changes go through the effect below.
  const font = useRef(fontFamily);
  font.current = fontFamily;
  const callbacks = useRef({ onStart, onEnd, onReader });
  callbacks.current = { onStart, onEnd, onReader };

  useEffect(() => {
    const element = container.current;
    if (!element) return;

    const darkScheme = window.matchMedia("(prefers-color-scheme: dark)");
    const terminal = new Terminal({
      // Required by the Unicode 11 addon, which gives emoji and CJK the widths
      // modern shells assume, so the cursor stays aligned while editing.
      allowProposedApi: true,
      cursorBlink: true,
      fontFamily: font.current,
      fontSize: 13,
      // Bounded, so a long-running process cannot grow memory without limit.
      scrollback: 10_000,
      theme: terminalTheme(darkScheme.matches),
    });
    terminalRef.current = terminal;
    const fit = new FitAddon();
    fitRef.current = fit;
    terminal.loadAddon(fit);
    terminal.loadAddon(new Unicode11Addon());
    terminal.unicode.activeVersion = "11";
    terminal.open(element);
    loadWebglRenderer(terminal);
    fit.fit();

    const session = new TerminalSession(
      native,
      terminal,
      {
        onStart: (info) => callbacks.current.onStart?.(info),
        onEnd: (ending) => callbacks.current.onEnd?.(ending),
      },
      { ...(program === undefined ? {} : { program }), once },
    );
    const resized = terminal.onResize(({ cols, rows }) => session.resize(cols, rows));
    session.start();

    let frame = 0;
    // Also fires when a hidden tab becomes visible, so it fits its space again.
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => fit.fit());
    });
    observer.observe(element);

    const applyScheme = () => (terminal.options.theme = terminalTheme(darkScheme.matches));
    darkScheme.addEventListener("change", applyScheme);

    let lastOutput = Date.now();
    const parsed = terminal.onWriteParsed(() => (lastOutput = Date.now()));
    callbacks.current.onReader?.({
      read: (lines) => {
        const buffer = terminal.buffer.active;
        const shown: string[] = [];
        for (let i = Math.max(0, buffer.length - lines); i < buffer.length; i++) {
          shown.push(buffer.getLine(i)?.translateToString(true) ?? "");
        }
        return shown.join("\n");
      },
      acceptsPaste: () => terminal.modes.bracketedPasteMode,
      paste: (text) => {
        // Without bracketed paste a newline in the text would press Enter.
        if (!terminal.modes.bracketedPasteMode) return false;
        terminal.paste(text);
        return true;
      },
      quietFor: () => Date.now() - lastOutput,
    });

    return () => {
      callbacks.current.onReader?.(null);
      parsed.dispose();
      darkScheme.removeEventListener("change", applyScheme);
      observer.disconnect();
      cancelAnimationFrame(frame);
      resized.dispose();
      session.dispose();
      terminal.dispose();
      terminalRef.current = null;
      fitRef.current = null;
    };
  }, [native, program, once]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || terminal.options.fontFamily === fontFamily) return;
    terminal.options.fontFamily = fontFamily;
    // The cells change size with the font.
    fitRef.current?.fit();
  }, [fontFamily]);

  useEffect(() => {
    if (visible && focused) terminalRef.current?.focus();
  }, [visible, focused, focusRequest]);

  return <div className="terminal" ref={container} />;
}

/** The GPU renderer copes with heavy output far better than the DOM renderer. */
function loadWebglRenderer(terminal: Terminal): void {
  try {
    const webgl = new WebglAddon();
    // Losing the GPU context falls back to the DOM renderer.
    webgl.onContextLoss(() => webgl.dispose());
    terminal.loadAddon(webgl);
  } catch (error) {
    console.warn("WebGL renderer unavailable; using the DOM renderer", error);
  }
}
