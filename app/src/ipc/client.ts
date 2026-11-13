// Typed wrappers around the Tauri commands in app/src-tauri/src/commands.rs.
// Payload types are generated from Rust (src/bindings, `npm run bindings`);
// only the command names and argument shapes are written by hand here.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { AppInfo } from "../bindings/AppInfo";
import type { CaptureReport } from "../bindings/CaptureReport";
import type { CaptureRequest } from "../bindings/CaptureRequest";
import type { CommandError } from "../bindings/CommandError";
import type { IconUpdate } from "../bindings/IconUpdate";
import type { RestoreProgress } from "../bindings/RestoreProgress";
import type { RestoreReport } from "../bindings/RestoreReport";
import type { RestoreRequest } from "../bindings/RestoreRequest";
import type { ShortcutCreated } from "../bindings/ShortcutCreated";
import type { SnapshotDetail } from "../bindings/SnapshotDetail";
import type { SnapshotEntry } from "../bindings/SnapshotEntry";
import type { WorkspaceIcon } from "../bindings/WorkspaceIcon";
import type { WorkspaceSummary } from "../bindings/WorkspaceSummary";

export const RESTORE_PROGRESS_EVENT = "restore://progress";

/** Every rejected command is normalised to a `CommandError`. */
export function toCommandError(e: unknown): CommandError {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    return e as CommandError;
  }
  return { code: "internal", message: e instanceof Error ? e.message : String(e) };
}

async function call<T>(
  cmd: string,
  args?: Record<string, unknown> | Uint8Array,
  options?: { headers: Record<string, string> },
): Promise<T> {
  try {
    return await invoke<T>(cmd, args, options);
  } catch (e) {
    throw toCommandError(e);
  }
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),

  listWorkspaces: (includeArchived = false) =>
    call<WorkspaceSummary[]>("list_workspaces", { includeArchived }),

  createWorkspace: (name: string, description?: string) =>
    call<WorkspaceSummary>("create_workspace", { name, description: description ?? null }),

  listSnapshots: (workspace: string) =>
    call<SnapshotEntry[]>("list_snapshots", { workspace }),

  getSnapshot: (workspace: string, version?: number) =>
    call<SnapshotDetail>("get_snapshot", { workspace, version: version ?? null }),

  capture: (request: CaptureRequest) =>
    call<CaptureReport>("capture_workspace", { request }),

  restore: (request: RestoreRequest) =>
    call<RestoreReport>("restore_workspace", { request }),

  cancelRestore: () => call<boolean>("cancel_restore"),

  /** Desktop icon that restores the workspace; rejects with `already_exists` unless `overwrite`. */
  createShortcut: (workspace: string, overwrite = false) =>
    call<ShortcutCreated>("create_shortcut", { workspace, overwrite }),

  getWorkspaceIcon: (workspace: string) => call<WorkspaceIcon>("get_workspace_icon", { workspace }),

  /** Sent as raw bytes; Rust converts it to a multi-size .ico and updates the desktop icon. */
  setWorkspaceIcon: (workspace: string, picture: Uint8Array) =>
    call<IconUpdate>("set_workspace_icon", picture, {
      headers: { "x-workspace": encodeURIComponent(workspace) },
    }),

  resetWorkspaceIcon: (workspace: string) => call<IconUpdate>("reset_workspace_icon", { workspace }),

  onRestoreProgress: (handler: (p: RestoreProgress) => void): Promise<UnlistenFn> =>
    listen<RestoreProgress>(RESTORE_PROGRESS_EVENT, (e) => handler(e.payload)),
};

export type {
  AppInfo,
  CaptureReport,
  CommandError,
  IconUpdate,
  WorkspaceIcon,
  RestoreProgress,
  RestoreReport,
  SnapshotDetail,
  SnapshotEntry,
  WorkspaceSummary,
};
