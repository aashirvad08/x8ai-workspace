import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";

import type { NativeClient } from "../native";
import { TerminalSession } from "./session";
import { terminalTheme } from "./theme";

/**
 * Renders one terminal session with xterm.js. This component only wires the
 * emulator to the DOM; session behaviour lives in `TerminalSession`.
 */
export function TerminalView({ native }: { native: NativeClient }) {
  const container = useRef<HTMLDivElement>(null);

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
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.loadAddon(new Unicode11Addon());
    terminal.unicode.activeVersion = "11";
    terminal.open(element);
    loadWebglRenderer(terminal);
    fit.fit();

    const session = new TerminalSession(native, terminal);
    const resized = terminal.onResize(({ cols, rows }) => session.resize(cols, rows));
    session.start();
    terminal.focus();

    let frame = 0;
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
    };
  }, [native]);

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
