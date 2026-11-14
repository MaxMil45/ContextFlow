// Typed wrappers around the Tauri commands in app/src-tauri/src/commands.rs.
// Payload types are generated from Rust (src/bindings, `npm run bindings`);
// only the command names and argument shapes are written by hand here.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { AppInfo } from "../bindings/AppInfo";
import type { CaptureReport } from "../bindings/CaptureReport";
import type { CaptureRequest } from "../bindings/CaptureRequest";
import type { BrowserStatus } from "../bindings/BrowserStatus";
import type { CaptureDiscovery } from "../bindings/CaptureDiscovery";
import type { CleanupConfig } from "../bindings/CleanupConfig";
import type { CleanupPhase } from "../bindings/CleanupPhase";
import type { CleanupPlan } from "../bindings/CleanupPlan";
import type { CleanupReport } from "../bindings/CleanupReport";
import type { CleanupTarget } from "../bindings/CleanupTarget";
import type { CloseMode } from "../bindings/CloseMode";
import type { CommandError } from "../bindings/CommandError";
import type { CompareResult } from "../bindings/CompareResult";
import type { DiffStatus } from "../bindings/DiffStatus";
import type { FileResult } from "../bindings/FileResult";
import type { ImportedWorkspace } from "../bindings/ImportedWorkspace";
import type { PlanItem } from "../bindings/PlanItem";
import type { RetentionResult } from "../bindings/RetentionResult";
import type { RunningApp } from "../bindings/RunningApp";
import type { Settings } from "../bindings/Settings";
import type { VerifyResult } from "../bindings/VerifyResult";
import type { DeleteOutcome } from "../bindings/DeleteOutcome";
import type { LeaveReport } from "../bindings/LeaveReport";
import type { HotkeyStatusDto } from "../bindings/HotkeyStatusDto";
import type { RestoreRunEntry } from "../bindings/RestoreRunEntry";
import type { WorkspaceUpdate } from "../bindings/WorkspaceUpdate";
import type { IconUpdate } from "../bindings/IconUpdate";
import type { RestoreProgress } from "../bindings/RestoreProgress";
import type { RestoreReport } from "../bindings/RestoreReport";
import type { RestoreRequest } from "../bindings/RestoreRequest";
import type { ShortcutCreated } from "../bindings/ShortcutCreated";
import type { TrustReview } from "../bindings/TrustReview";
import type { SnapshotDetail } from "../bindings/SnapshotDetail";
import type { SnapshotEntry } from "../bindings/SnapshotEntry";
import type { WorkspaceIcon } from "../bindings/WorkspaceIcon";
import type { WorkspaceSummary } from "../bindings/WorkspaceSummary";

export const RESTORE_PROGRESS_EVENT = "restore://progress";
export const TRAY_CAPTURE_EVENT = "tray://capture";
export const TRAY_RESTORE_EVENT = "tray://restore";
export const CAPTURE_SAVED_EVENT = "capture://saved";

/** What the on-screen confirmation card shows (sent by the Rust side). */

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

  /** Save the workspace you're in, leave it, then restore this one. */
  switchWorkspace: (request: RestoreRequest) =>
    call<RestoreReport>("switch_workspace", { request }),

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

  /** Change name and description together; desktop icons follow a rename. */
  updateWorkspace: (workspace: string, name: string, description: string | null) =>
    call<WorkspaceUpdate>("update_workspace", { workspace, name, description }),

  /** The workspace's recent restores, newest first. */
  listRestoreRuns: (workspace: string) => call<RestoreRunEntry[]>("list_restore_runs", { workspace }),

  setWorkspaceArchived: (workspace: string, archived: boolean) =>
    call<WorkspaceSummary>("set_workspace_archived", { workspace, archived }),

  /** Desktop icons (full paths) that restore this workspace. */
  getWorkspaceShortcuts: (workspace: string) =>
    call<string[]>("get_workspace_shortcuts", { workspace }),

  /** Permanently deletes the workspace and all its snapshots; icons go to the Recycle Bin if asked. */
  deleteWorkspace: (workspace: string, deleteShortcuts: boolean) =>
    call<DeleteOutcome>("delete_workspace", { workspace, deleteShortcuts }),

  /** `null` = restore the latest snapshot again. */
  setPinnedVersion: (workspace: string, version: number | null) =>
    call<WorkspaceSummary>("set_pinned_version", { workspace, version }),

  deleteSnapshot: (workspace: string, version: number) =>
    call<void>("delete_snapshot", { workspace, version }),

  getTrustReview: (workspace: string) => call<TrustReview>("get_trust_review", { workspace }),

  approveWorkspace: (workspace: string) => call<WorkspaceSummary>("approve_workspace", { workspace }),

  /** Programs running now with memory and CPU (takes about a second). */
  listRunningApps: () => call<RunningApp[]>("list_running_apps"),

  getCleanupConfig: (workspace: string) => call<CleanupConfig>("get_cleanup_config", { workspace }),

  setCleanupConfig: (workspace: string, config: CleanupConfig) =>
    call<CleanupConfig>("set_cleanup_config", { workspace, config }),

  /** What one side of the cleanup would close right now, and what that frees. */
  previewCleanup: (workspace: string, phase: CleanupPhase, fresh = false) =>
    call<CleanupPlan>("preview_cleanup", { workspace, phase, fresh }),

  /** Closes the programs now (the UI confirms first). */
  runCleanupNow: (workspace: string, phase: CleanupPhase) =>
    call<CleanupReport>("run_cleanup_now", { workspace, phase }),

  /** Leave the active workspace, running its "when leaving" cleanup. */
  leaveWorkspace: () => call<LeaveReport>("leave_workspace"),

  // --- capture review and compare
  discoverCapture: (workspace: string) => call<CaptureDiscovery>("discover_capture", { workspace }),

  /** `to` omitted = compare with the screen right now. */
  compareSnapshots: (workspace: string, from: number, to?: number) =>
    call<CompareResult>("compare_snapshots", { workspace, from, to: to ?? null }),

  // --- settings and data
  getSettings: () => call<Settings>("get_settings"),
  /** Which global shortcuts work; a message for each one that is taken. */
  getHotkeyStatus: () => call<HotkeyStatusDto>("get_hotkey_status"),
  saveSettings: (settings: Settings) => call<Settings>("save_settings", { settings }),
  getBrowserStatus: () => call<BrowserStatus>("get_browser_status"),
  installBrowserIntegration: () => call<BrowserStatus>("install_browser_integration"),
  removeBrowserIntegration: () => call<BrowserStatus>("remove_browser_integration"),
  openFolder: (which: "extension" | "data" | "logs" | "backups") => call<void>("open_folder", { which }),
  /** Each of these opens a Windows file dialog; `null` = the user cancelled it. */
  backupNow: () => call<FileResult | null>("backup_now"),
  exportWorkspaces: (workspaces?: string[]) =>
    call<FileResult | null>("export_workspaces", { workspaces: workspaces ?? null }),
  importWorkspaces: () => call<ImportedWorkspace[] | null>("import_workspaces"),
  verifyData: () => call<VerifyResult>("verify_data"),
  previewRetention: () => call<RetentionResult>("preview_retention"),
  applyRetention: () => call<RetentionResult>("apply_retention"),
  refreshTrayMenu: () => call<void>("refresh_tray_menu"),

  /** A quick capture (the shortcut, or the tray's Capture now) saved a snapshot of this workspace. */
  onCaptureSaved: (handler: (workspace: string) => void): Promise<UnlistenFn> =>
    listen<string>(CAPTURE_SAVED_EVENT, (e) => handler(e.payload)),


  onTrayCapture: (handler: () => void): Promise<UnlistenFn> => listen(TRAY_CAPTURE_EVENT, () => handler()),
  onTrayRestore: (handler: (workspace: string) => void): Promise<UnlistenFn> =>
    listen<string>(TRAY_RESTORE_EVENT, (e) => handler(e.payload)),

  /** A switch from the switcher needs attention: show its report. */
  onSwitchReport: (handler: (r: RestoreReport) => void): Promise<UnlistenFn> =>
    listen<RestoreReport>("switch://report", (e) => handler(e.payload)),

  onRestoreProgress: (handler: (p: RestoreProgress) => void): Promise<UnlistenFn> =>
    listen<RestoreProgress>(RESTORE_PROGRESS_EVENT, (e) => handler(e.payload)),
};

export type {
  LeaveReport,
  RestoreRunEntry,
  WorkspaceUpdate,
  BrowserStatus,
  CaptureDiscovery,
  DiffStatus,
  FileResult,
  ImportedWorkspace,
  RetentionResult,
  Settings,
  VerifyResult,
  AppInfo,
  CaptureReport,
  CleanupConfig,
  CleanupPhase,
  CleanupPlan,
  CleanupReport,
  CleanupTarget,
  CloseMode,
  CommandError,
  CompareResult,
  PlanItem,
  RunningApp,
  DeleteOutcome,
  IconUpdate,
  WorkspaceIcon,
  RestoreProgress,
  RestoreReport,
  TrustReview,
  SnapshotDetail,
  SnapshotEntry,
  WorkspaceSummary,
};
