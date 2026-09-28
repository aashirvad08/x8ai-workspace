import type { TerminalEvent } from "../contracts/generated/TerminalEvent";
import type { TerminalExit } from "../contracts/generated/TerminalExit";
import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import type { TerminalApi } from "../native";

interface Disposable {
  dispose(): void;
}

/** The part of an xterm.js `Terminal` a session drives. */
export interface TerminalScreen {
  readonly cols: number;
  readonly rows: number;
  write(data: string | Uint8Array, callback?: () => void): void;
  onData(listener: (data: string) => void): Disposable;
  onBinary(listener: (data: string) => void): Disposable;
}

/** Optional notifications, e.g. for a tab title. */
export interface SessionCallbacks {
  /** A shell started (again, after a restart). */
  onStart?(info: TerminalInfo): void;
  /** The shell exited, or could not start. */
  onEnd?(): void;
}

/** One native session: from creation until it exits or is replaced. */
interface Attempt {
  info: TerminalInfo | undefined;
  /** The process exited, or never started. Enter starts a new one. */
  over: boolean;
  /** Input typed before the session was ready. */
  queued: Uint8Array[];
  /** Output bytes rendered, and how many of them have been acknowledged. */
  rendered: number;
  acked: number;
  /** The size the PTY has. */
  cols: number;
  rows: number;
}

const encoder = new TextEncoder();
const ENTER = "\r";

/**
 * Connects a terminal screen to a native session running the user's login shell.
 * Input goes to the PTY, output is rendered with flow control, resizes are
 * forwarded, and when the shell exits, Enter starts a new one.
 *
 * Holds no process logic (that lives in Rust) and no React.
 */
export class TerminalSession {
  readonly #native: TerminalApi;
  readonly #screen: TerminalScreen;
  readonly #callbacks: SessionCallbacks;
  readonly #subscriptions: Disposable[];
  #attempt: Attempt | undefined;
  #disposed = false;

  constructor(native: TerminalApi, screen: TerminalScreen, callbacks: SessionCallbacks = {}) {
    this.#native = native;
    this.#screen = screen;
    this.#callbacks = callbacks;
    this.#subscriptions = [
      screen.onData((data) => this.#input(data, encoder.encode(data))),
      // Binary data is a string of byte values, used by some mouse reports.
      screen.onBinary((data) => this.#input(data, Uint8Array.from(data, (c) => c.charCodeAt(0) & 0xff))),
    ];
  }

  /** Starts a new shell at the screen's current size. */
  start(): void {
    if (this.#disposed) return;
    const attempt: Attempt = {
      info: undefined,
      over: false,
      queued: [],
      rendered: 0,
      acked: 0,
      cols: this.#screen.cols,
      rows: this.#screen.rows,
    };
    this.#attempt = attempt;

    const listener = {
      output: (data: Uint8Array) => this.#output(attempt, data),
      event: (event: TerminalEvent) => this.#event(attempt, event),
    };
    this.#native.createTerminal({ cols: attempt.cols, rows: attempt.rows }, listener).then(
      (info) => this.#ready(attempt, info),
      (error: unknown) => {
        if (!this.#isCurrent(attempt)) return;
        attempt.over = true;
        this.#callbacks.onEnd?.();
        this.#report("Could not start the shell", error);
        this.#notice("press Enter to try again");
      },
    );
  }

  /** Forwards a new screen size to the PTY. */
  resize(cols: number, rows: number): void {
    const attempt = this.#attempt;
    // Until the session is ready, the screen's size is checked once it is.
    if (!attempt?.info || attempt.over || (attempt.cols === cols && attempt.rows === rows)) return;
    attempt.cols = cols;
    attempt.rows = rows;
    this.#send(this.#native.resizeTerminal(attempt.info.id, { cols, rows }));
  }

  /** Hangs up the session and stops listening to the screen. */
  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const subscription of this.#subscriptions) subscription.dispose();
    const attempt = this.#attempt;
    this.#attempt = undefined;
    if (attempt?.info && !attempt.over) this.#close(attempt.info);
  }

  #isCurrent(attempt: Attempt): boolean {
    return !this.#disposed && attempt === this.#attempt;
  }

  #ready(attempt: Attempt, info: TerminalInfo): void {
    if (!this.#isCurrent(attempt)) {
      this.#close(info);
      return;
    }
    attempt.info = info;
    if (attempt.over) {
      // It exited before creation was acknowledged.
      this.#close(info);
      return;
    }
    this.#callbacks.onStart?.(info);
    for (const bytes of attempt.queued.splice(0)) this.#send(this.#native.writeTerminal(info.id, bytes));
    const { cols, rows } = this.#screen;
    if (cols !== attempt.cols || rows !== attempt.rows) {
      attempt.cols = cols;
      attempt.rows = rows;
      this.#send(this.#native.resizeTerminal(info.id, { cols, rows }));
    }
    this.#acknowledge(attempt);
  }

  #input(text: string, bytes: Uint8Array): void {
    const attempt = this.#attempt;
    if (!attempt || this.#disposed) return;
    if (attempt.over) {
      if (text.includes(ENTER)) this.start();
      return;
    }
    if (attempt.info) {
      this.#send(this.#native.writeTerminal(attempt.info.id, bytes));
    } else {
      attempt.queued.push(bytes);
    }
  }

  #output(attempt: Attempt, data: Uint8Array): void {
    if (!this.#isCurrent(attempt)) return;
    this.#screen.write(data, () => {
      attempt.rendered += data.length;
      if (this.#isCurrent(attempt)) this.#acknowledge(attempt);
    });
  }

  /** Acknowledges rendered output in steps of `ackBytes`, so the native side keeps reading. */
  #acknowledge(attempt: Attempt): void {
    const { info } = attempt;
    const pending = attempt.rendered - attempt.acked;
    if (!info || attempt.over || pending < info.ackBytes) return;
    attempt.acked = attempt.rendered;
    this.#send(this.#native.ackTerminal(info.id, pending));
  }

  #event(attempt: Attempt, event: TerminalEvent): void {
    if (!this.#isCurrent(attempt)) return;
    switch (event.type) {
      case "error":
        this.#notice(event.message, "error");
        break;
      case "exited":
        attempt.over = true;
        this.#callbacks.onEnd?.();
        // The process is gone; closing releases the native session.
        if (attempt.info) this.#close(attempt.info);
        this.#notice(`${describeExit(event)}, press Enter to start a new shell`);
        break;
      default:
        // Unreachable while the contracts and this switch agree; loud if they drift.
        console.error("Unexpected terminal event", event satisfies never);
    }
  }

  #close(info: TerminalInfo): void {
    this.#native.closeTerminal(info.id).catch((error: unknown) => {
      console.error(`Could not close terminal session ${info.id}`, error);
    });
  }

  /** Runs a native call whose failure should be visible but not fatal. */
  #send(call: Promise<void>): void {
    call.catch((error: unknown) => this.#report("Terminal error", error));
  }

  #report(context: string, error: unknown): void {
    console.error(context, error);
    const message = error instanceof Error ? error.message : String(error);
    this.#notice(`${context}: ${message}`, "error");
  }

  #notice(text: string, tone: "info" | "error" = "info"): void {
    if (this.#disposed) return;
    const style = tone === "error" ? "\x1b[31m" : "\x1b[2m";
    this.#screen.write(`\r\n${style}[${text}]\x1b[0m\r\n`);
  }
}

function describeExit({ code, signal }: TerminalExit): string {
  return signal ? `Process terminated (${signal})` : `Process exited with code ${code}`;
}
