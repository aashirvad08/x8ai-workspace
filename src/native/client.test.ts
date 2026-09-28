import { describe, expect, it } from "vitest";

import type { AppInfo } from "../contracts/generated/AppInfo";
import type { TerminalEvent } from "../contracts/generated/TerminalEvent";
import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import { createNativeClient, type Invoke, type InvokeArgs, type InvokeOptions, SESSION_ID_HEADER } from "./client";
import { NativeError } from "./errors";

const info: AppInfo = { name: "x8ai Workspace", version: "0.1.0", os: "macos", arch: "aarch64" };

interface Call {
  command: string;
  args: InvokeArgs | undefined;
  options: InvokeOptions | undefined;
}

/** A fake bridge that records invocations and exposes the channels it created. */
function bridge(respond: (call: Call) => unknown = () => undefined) {
  const calls: Call[] = [];
  const channels: Array<(message: unknown) => void> = [];
  const invoke: Invoke = async (command, args, options) => {
    const call = { command, args, options };
    calls.push(call);
    return respond(call);
  };
  const createChannel = (onMessage: (message: unknown) => void) => {
    channels.push(onMessage);
    return { channel: channels.length };
  };
  return { calls, channels, client: createNativeClient({ invoke, createChannel }) };
}

function rejectingWith(reason: unknown) {
  return createNativeClient({ invoke: () => Promise.reject(reason), createChannel: () => ({}) });
}

async function failure(promise: Promise<unknown>): Promise<NativeError> {
  const error = await promise.then(
    () => expect.unreachable("expected the call to fail"),
    (e: unknown) => e,
  );
  expect(error).toBeInstanceOf(NativeError);
  return error as NativeError;
}

describe("native client", () => {
  it("invokes the matching command and returns its result", async () => {
    const { calls, client } = bridge(() => info);

    await expect(client.getAppInfo()).resolves.toEqual(info);
    expect(calls.map((c) => c.command)).toEqual(["get_app_info"]);
  });

  it("preserves structured command errors", async () => {
    const client = rejectingWith({ code: "notFound", message: "no such session" });

    const error = await failure(client.getAppInfo());
    expect(error).toMatchObject({ command: "get_app_info", code: "notFound", message: "no such session" });
  });

  it("reports calls that never reached a command as ipc errors", async () => {
    // Tauri rejects with a plain string when a command is not granted to the window.
    const client = rejectingWith("get_app_info not allowed");

    const error = await failure(client.getAppInfo());
    expect(error).toMatchObject({ code: "ipc", message: "get_app_info not allowed" });
  });

  it("reports a missing bridge as an ipc error", async () => {
    const client = rejectingWith(new TypeError("window.__TAURI_INTERNALS__ is undefined"));

    const error = await failure(client.getAppInfo());
    expect(error.code).toBe("ipc");
  });

  it("does not trust unknown error codes", async () => {
    const client = rejectingWith({ code: "rootAccessGranted", message: "?" });

    const error = await failure(client.getAppInfo());
    expect(error.code).toBe("ipc");
  });
});

describe("native client terminal commands", () => {
  const created: TerminalInfo = { id: 7, program: "/bin/zsh", ackBytes: 65536 };

  it("creates a session with a channel and splits output from events", async () => {
    const { calls, channels, client } = bridge(() => created);
    const output: Uint8Array[] = [];
    const events: TerminalEvent[] = [];

    const result = await client.createTerminal(
      { cols: 80, rows: 24 },
      { output: (data) => output.push(data), event: (event) => events.push(event) },
    );

    expect(result).toEqual(created);
    expect(calls[0]).toMatchObject({
      command: "terminal_create",
      args: { size: { cols: 80, rows: 24 }, events: { channel: 1 } },
    });

    const deliver = channels[0]!;
    deliver(new Uint8Array([104, 105]).buffer);
    deliver({ type: "exited", code: 0, signal: null });
    expect(output).toEqual([new Uint8Array([104, 105])]);
    expect(events).toEqual([{ type: "exited", code: 0, signal: null }]);
  });

  it("writes input as a raw body addressed by header", async () => {
    const { calls, client } = bridge();
    const bytes = new Uint8Array([0x03]);

    await client.writeTerminal(7, bytes);

    expect(calls[0]).toEqual({
      command: "terminal_write",
      args: bytes,
      options: { headers: { [SESSION_ID_HEADER]: "7" } },
    });
  });

  it("sends resize, ack and close with named arguments", async () => {
    const { calls, client } = bridge();

    await client.resizeTerminal(7, { cols: 100, rows: 30 });
    await client.ackTerminal(7, 65536);
    await client.closeTerminal(7);

    expect(calls.map(({ command, args }) => ({ command, args }))).toEqual([
      { command: "terminal_resize", args: { id: 7, size: { cols: 100, rows: 30 } } },
      { command: "terminal_ack", args: { id: 7, bytes: 65536 } },
      { command: "terminal_close", args: { id: 7 } },
    ]);
  });
});
