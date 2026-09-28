import { describe, expect, it } from "vitest";

import type { SessionId } from "../contracts/generated/SessionId";
import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import type { TerminalSize } from "../contracts/generated/TerminalSize";
import type { NativeClient, TerminalListener } from "../native";
import { type TerminalScreen, TerminalSession } from "./session";

/** A screen that renders synchronously and lets the test type into it. */
class FakeScreen implements TerminalScreen {
  cols = 80;
  rows = 24;
  written: Array<string | Uint8Array> = [];
  #onData: Array<(data: string) => void> = [];
  #onBinary: Array<(data: string) => void> = [];

  write(data: string | Uint8Array, callback?: () => void): void {
    this.written.push(data);
    callback?.();
  }
  onData(listener: (data: string) => void) {
    this.#onData.push(listener);
    return { dispose: () => (this.#onData = this.#onData.filter((l) => l !== listener)) };
  }
  onBinary(listener: (data: string) => void) {
    this.#onBinary.push(listener);
    return { dispose: () => (this.#onBinary = this.#onBinary.filter((l) => l !== listener)) };
  }
  type(data: string) {
    for (const listener of this.#onData) listener(data);
  }
  typeBinary(data: string) {
    for (const listener of this.#onBinary) listener(data);
  }
  text(): string {
    return this.written.filter((w): w is string => typeof w === "string").join("");
  }
}

interface Created {
  size: TerminalSize;
  listener: TerminalListener;
  resolve(info: TerminalInfo): void;
  reject(error: unknown): void;
}

type Recorded =
  | { call: "write"; id: SessionId; data: number[] }
  | { call: "resize"; id: SessionId; size: TerminalSize }
  | { call: "ack"; id: SessionId; bytes: number }
  | { call: "close"; id: SessionId };

/** A native client whose sessions the test creates, feeds and finishes by hand. */
class FakeNative implements NativeClient {
  created: Created[] = [];
  calls: Recorded[] = [];

  getAppInfo(): never {
    throw new Error("not used");
  }
  createTerminal(size: TerminalSize, listener: TerminalListener): Promise<TerminalInfo> {
    return new Promise((resolve, reject) => this.created.push({ size, listener, resolve, reject }));
  }
  async writeTerminal(id: SessionId, data: Uint8Array) {
    this.calls.push({ call: "write", id, data: [...data] });
  }
  async resizeTerminal(id: SessionId, size: TerminalSize) {
    this.calls.push({ call: "resize", id, size });
  }
  async ackTerminal(id: SessionId, bytes: number) {
    this.calls.push({ call: "ack", id, bytes });
  }
  async closeTerminal(id: SessionId) {
    this.calls.push({ call: "close", id });
  }
}

const info = (id: number, ackBytes = 1024): TerminalInfo => ({ id, program: "/bin/zsh", ackBytes });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

async function running(ackBytes?: number) {
  const native = new FakeNative();
  const screen = new FakeScreen();
  const session = new TerminalSession(native, screen);
  session.start();
  native.created[0]!.resolve(info(1, ackBytes));
  await settle();
  return { native, screen, session, listener: native.created[0]!.listener };
}

describe("TerminalSession", () => {
  it("starts a shell at the screen's size and renders its output", async () => {
    const { native, screen, listener } = await running();

    expect(native.created[0]!.size).toEqual({ cols: 80, rows: 24 });
    listener.output(new TextEncoder().encode("hello"));
    expect(screen.written).toEqual([new TextEncoder().encode("hello")]);
  });

  it("sends typed text as UTF-8 and binary input as raw bytes", async () => {
    const { native, screen } = await running();

    screen.type("é\x03");
    screen.typeBinary("\x1b[M\x80");

    expect(native.calls).toEqual([
      { call: "write", id: 1, data: [0xc3, 0xa9, 0x03] },
      { call: "write", id: 1, data: [0x1b, 0x5b, 0x4d, 0x80] },
    ]);
  });

  it("delivers input typed before the shell is ready, in order", async () => {
    const native = new FakeNative();
    const screen = new FakeScreen();
    new TerminalSession(native, screen).start();

    screen.type("ls");
    screen.type("\r");
    expect(native.calls).toEqual([]);

    native.created[0]!.resolve(info(1));
    await settle();
    expect(native.calls).toEqual([
      { call: "write", id: 1, data: [0x6c, 0x73] },
      { call: "write", id: 1, data: [0x0d] },
    ]);
  });

  it("acknowledges rendered output in ackBytes steps", async () => {
    const { native, listener } = await running(10);

    listener.output(new Uint8Array(6));
    expect(native.calls).toEqual([]);
    listener.output(new Uint8Array(6));
    listener.output(new Uint8Array(3));
    expect(native.calls).toEqual([{ call: "ack", id: 1, bytes: 12 }]);
  });

  it("acknowledges output that arrived before the session was ready", async () => {
    const native = new FakeNative();
    new TerminalSession(native, new FakeScreen()).start();

    native.created[0]!.listener.output(new Uint8Array(2048));
    native.created[0]!.resolve(info(1, 1024));
    await settle();
    expect(native.calls).toEqual([{ call: "ack", id: 1, bytes: 2048 }]);
  });

  it("forwards size changes once, including one made while starting", async () => {
    const native = new FakeNative();
    const screen = new FakeScreen();
    const session = new TerminalSession(native, screen);
    session.start();
    screen.cols = 100;
    session.resize(100, 24);
    native.created[0]!.resolve(info(1));
    await settle();

    session.resize(100, 24);
    session.resize(120, 40);
    expect(native.calls).toEqual([
      { call: "resize", id: 1, size: { cols: 100, rows: 24 } },
      { call: "resize", id: 1, size: { cols: 120, rows: 40 } },
    ]);
  });

  it("reports an exit, releases the session and restarts on Enter", async () => {
    const { native, screen, listener } = await running();

    listener.event({ type: "exited", code: 0, signal: null });
    expect(screen.text()).toContain("Process exited with code 0");
    expect(native.calls).toEqual([{ call: "close", id: 1 }]);

    screen.type("x");
    expect(native.created).toHaveLength(1);
    screen.type("\r");
    expect(native.created).toHaveLength(2);
  });

  it("describes a process killed by a signal", async () => {
    const { screen, listener } = await running();
    listener.event({ type: "exited", code: 1, signal: "Hangup: 1" });
    expect(screen.text()).toContain("Process terminated (Hangup: 1)");
  });

  it("shows errors from the native side", async () => {
    const { screen, listener } = await running();
    listener.event({ type: "error", message: "reading terminal output failed" });
    expect(screen.text()).toContain("reading terminal output failed");
  });

  it("ignores output from a session it has replaced", async () => {
    const { native, screen, listener } = await running();
    listener.event({ type: "exited", code: 0, signal: null });
    screen.type("\r");
    const before = screen.written.length;

    listener.output(new Uint8Array([1, 2, 3]));
    expect(screen.written).toHaveLength(before);
    expect(native.created).toHaveLength(2);
  });

  it("shows a failure to start and retries on Enter", async () => {
    const native = new FakeNative();
    const screen = new FakeScreen();
    new TerminalSession(native, screen).start();
    native.created[0]!.reject(new Error("failed to start the terminal: no shell"));
    await settle();

    expect(screen.text()).toContain("Could not start the shell: failed to start the terminal: no shell");
    screen.type("\r");
    expect(native.created).toHaveLength(2);
  });

  it("closes the session when disposed and stops listening", async () => {
    const { native, screen, session } = await running();

    session.dispose();
    screen.type("ls\r");
    expect(native.calls).toEqual([{ call: "close", id: 1 }]);
  });

  it("closes a session that finishes starting after disposal", async () => {
    const native = new FakeNative();
    const session = new TerminalSession(native, new FakeScreen());
    session.start();
    session.dispose();

    native.created[0]!.resolve(info(1));
    await settle();
    expect(native.calls).toEqual([{ call: "close", id: 1 }]);
  });
});
