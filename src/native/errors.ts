import type { CommandError } from "../contracts/generated/CommandError";
import type { ErrorCode } from "../contracts/generated/ErrorCode";

/**
 * A command's own error codes, plus `ipc` when the call never reached a command:
 * it is not registered, not granted to this window, its arguments did not
 * deserialize, or the Tauri bridge is missing (e.g. running in a plain browser).
 */
export type NativeErrorCode = ErrorCode | "ipc";

// A Record forces this list to change whenever the Rust `ErrorCode` enum does.
const COMMAND_ERROR_CODES: Record<ErrorCode, true> = {
  invalidInput: true,
  notFound: true,
  permissionDenied: true,
  internal: true,
};

export class NativeError extends Error {
  override readonly name = "NativeError";
  readonly command: string;
  readonly code: NativeErrorCode;

  constructor(command: string, code: NativeErrorCode, message: string) {
    super(message);
    this.command = command;
    this.code = code;
  }

  /** Normalizes whatever `invoke` rejected with. */
  static from(command: string, reason: unknown): NativeError {
    if (isCommandError(reason)) {
      return new NativeError(command, reason.code, reason.message);
    }
    return new NativeError(command, "ipc", describe(reason));
  }
}

function isCommandError(value: unknown): value is CommandError {
  if (typeof value !== "object" || value === null) return false;
  const { code, message } = value as Record<string, unknown>;
  return typeof code === "string" && Object.hasOwn(COMMAND_ERROR_CODES, code) && typeof message === "string";
}

function describe(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  try {
    return JSON.stringify(reason) ?? String(reason);
  } catch {
    return String(reason);
  }
}
