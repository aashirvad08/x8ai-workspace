import { describe, expect, it } from "vitest";

import type { AppInfo } from "../contracts/generated/AppInfo";
import { createNativeClient, type Invoke } from "./client";
import { NativeError } from "./errors";

const info: AppInfo = { name: "x8ai Workspace", version: "0.1.0", os: "macos", arch: "aarch64" };

function rejectingWith(reason: unknown): Invoke {
  return () => Promise.reject(reason);
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
    const calls: string[] = [];
    const invoke: Invoke = async (command) => {
      calls.push(command);
      return info;
    };

    await expect(createNativeClient(invoke).getAppInfo()).resolves.toEqual(info);
    expect(calls).toEqual(["get_app_info"]);
  });

  it("preserves structured command errors", async () => {
    const client = createNativeClient(rejectingWith({ code: "notFound", message: "no such session" }));

    const error = await failure(client.getAppInfo());
    expect(error).toMatchObject({ command: "get_app_info", code: "notFound", message: "no such session" });
  });

  it("reports calls that never reached a command as ipc errors", async () => {
    // Tauri rejects with a plain string when a command is not granted to the window.
    const client = createNativeClient(rejectingWith("get_app_info not allowed"));

    const error = await failure(client.getAppInfo());
    expect(error).toMatchObject({ code: "ipc", message: "get_app_info not allowed" });
  });

  it("reports a missing bridge as an ipc error", async () => {
    const client = createNativeClient(rejectingWith(new TypeError("window.__TAURI_INTERNALS__ is undefined")));

    const error = await failure(client.getAppInfo());
    expect(error.code).toBe("ipc");
  });

  it("does not trust unknown error codes", async () => {
    const client = createNativeClient(rejectingWith({ code: "rootAccessGranted", message: "?" }));

    const error = await failure(client.getAppInfo());
    expect(error.code).toBe("ipc");
  });
});
