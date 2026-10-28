import { invoke } from "@tauri-apps/api/core";
import {
  FILE_COMMAND,
  FILE_NAME_HEADER,
  IMPORT_COMMAND,
  type CommandError,
  type ImportOutcome,
} from "@ledgerlens/bindings";

export { commands } from "@ledgerlens/bindings";

/** True inside the Tauri shell; the plain browser build has no backend to talk to. */
export const hasBackend =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type CommandResult<T> = { status: "ok"; data: T } | { status: "error"; error: CommandError };

/** Turns a generated command's result into a value, throwing its `CommandError`. */
export async function unwrap<T>(pending: Promise<CommandResult<T>>): Promise<T> {
  const result = await pending;
  if (result.status === "error") throw result.error;
  return result.data;
}

/**
 * Android's WebView can't send request bodies, so Tauri serialises invokes there as JSON, where
 * raw bytes would become a number per byte. There the file goes as one base64 string instead.
 */
const bodiesTravelAsJson =
  typeof navigator !== "undefined" && /\bAndroid\b/i.test(navigator.userAgent);

/** Base64 without the `data:` prefix, encoded natively by the WebView. */
function readBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error ?? new Error("Couldn't read the file"));
    reader.readAsDataURL(file);
  });
}

export async function importFile(file: File): Promise<ImportOutcome> {
  const body = bodiesTravelAsJson
    ? { base64: await readBase64(file) }
    : new Uint8Array(await file.arrayBuffer());
  return invoke<ImportOutcome>(IMPORT_COMMAND, body, {
    headers: { [FILE_NAME_HEADER]: encodeURIComponent(file.name) },
  });
}

export async function fileBytes(id: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>(FILE_COMMAND, { id });
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as CommandError).message);
  }
  return "Something went wrong.";
}
