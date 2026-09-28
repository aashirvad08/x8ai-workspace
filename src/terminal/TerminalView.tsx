import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";

import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import type { TerminalApi } from "../native";
import { TerminalSession } from "./session";
import { terminalTheme } from "./theme";

interface Props {
  native: TerminalApi;
  /** Hidden views keep running; only the active one is shown. */
  active: boolean;
  /** Focuses the terminal when this changes while it is active. */
  focusRequest: number;
  onStart?: (info: TerminalInfo) => void;
  onEnd?: () => void;
}

/**
 * Renders one terminal session with xterm.js. This component only wires the
 * emulator to the DOM; session behaviour lives in `TerminalSession`.
 */
export function TerminalView({ native, active, focusRequest, onStart, onEnd }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const callbacks = useRef({ onStart, onEnd });
  callbacks.current = { onStart, onEnd };

  useEffect(() => {
    const element = container.current;
    if (!element) return;

    const darkScheme = window.matchMedia("(prefers-color-scheme: dark)");
    const terminal = new Terminal({
      // Required by the Unicode 11 addon, which gives emoji and CJK the widths
      // modern shells assume, so the cursor stays aligned while editing.
      allowProposedApi: true,
      cursorBlink: true,
      fontFamily: '"SF Mono", Menlo, Monaco, monospace',
      fontSize: 13,
      // Bounded, so a long-running process cannot grow memory without limit.
      scrollback: 10_000,
      theme: terminalTheme(darkScheme.matches),
    });
    terminalRef.current = terminal;
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.loadAddon(new Unicode11Addon());
    terminal.unicode.activeVersion = "11";
    terminal.open(element);
    loadWebglRenderer(terminal);
    fit.fit();

    const session = new TerminalSession(native, terminal, {
      onStart: (info) => callbacks.current.onStart?.(info),
      onEnd: () => callbacks.current.onEnd?.(),
    });
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

    return () => {
      darkScheme.removeEventListener("change", applyScheme);
      observer.disconnect();
      cancelAnimationFrame(frame);
      resized.dispose();
      session.dispose();
      terminal.dispose();
      terminalRef.current = null;
    };
  }, [native]);

  useEffect(() => {
    if (active) terminalRef.current?.focus();
  }, [active, focusRequest]);

  return <div className="terminal" ref={container} hidden={!active} />;
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
