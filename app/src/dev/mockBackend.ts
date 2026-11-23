// Sample data for working on the interface in an ordinary browser, without the
// Rust side: `npm run dev`, then open http://localhost:1420/?mock
// Only loaded in development (see main.tsx); never part of a build.

import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import type {
  AppInfo,
  BrowserStatus,
  CleanupConfig,
  CleanupPlan,
  RestoreRunEntry,
  Settings,
  SnapshotDetail,
  SnapshotEntry,
  WorkspaceCommandsDto,
  WorkspaceSummary,
} from "../ipc/client";

const ago = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();

const entry = (version: number, minutes: number, auto = false): SnapshotEntry => ({
  auto,
  version,
  createdAt: ago(minutes),
  shortHash: `9f3c2a${version}d81be`,
  captureStatus: "complete",
});

const history: Record<string, SnapshotEntry[]> = {
  "5d0c6a2e-8b1f-4a37-9e2d-1f6b3c4a7e90": [entry(4, 2, true), entry(3, 90), entry(2, 60 * 26), entry(1, 60 * 24 * 6)],
  "a93e17f4-2c5b-4d8e-b061-7c2f9e3d5a18": [entry(2, 60 * 20), entry(1, 60 * 24 * 2)],
  "1e7b4c90-d6a3-4f25-8b1e-93c0a5f2d764": [entry(1, 60 * 23)],
  "c4f2e8a1-7d3b-49c6-a5e0-2b8d1f6c9a37": [],
};

const workspace = (id: string, name: string, extra: Partial<WorkspaceSummary> = {}): WorkspaceSummary => ({
  id,
  name,
  description: null,
  updatedAt: history[id]?.[0]?.createdAt ?? ago(5),
  isArchived: false,
  isTrusted: true,
  pinnedVersion: null,
  latest: history[id]?.[0] ?? null,
  isActive: false,
  saveMode: "auto",
  ...extra,
});

const workspaces: WorkspaceSummary[] = [
  workspace("5d0c6a2e-8b1f-4a37-9e2d-1f6b3c4a7e90", "Spider", { isActive: true, description: "The crawler project" }),
  workspace("a93e17f4-2c5b-4d8e-b061-7c2f9e3d5a18", "Uni", { saveMode: "notify" }),
  workspace("1e7b4c90-d6a3-4f25-8b1e-93c0a5f2d764", "Chess", { pinnedVersion: 1, saveMode: "fixed" }),
  workspace("c4f2e8a1-7d3b-49c6-a5e0-2b8d1f6c9a37", "Brand new workspace"),
];

const detail = (id: string, version?: number): SnapshotDetail => {
  const e = (version ? history[id]?.find((s) => s.version === version) : history[id]?.[0]) ?? entry(1, 5);
  return {
    ...e,
    notes: id === "5d0c6a2e-8b1f-4a37-9e2d-1f6b3c4a7e90" ? ["Fixing the retry logic in the fetcher"] : [],
    monitorCount: 2,
    applications: [
      { applicationId: "code", windowCount: 2, tabCount: 0, folderCount: 2 },
      { applicationId: "chrome", windowCount: 1, tabCount: 7, folderCount: 0 },
      { applicationId: "windowsterminal", windowCount: 1, tabCount: 0, folderCount: 2 },
      { applicationId: "explorer", windowCount: 1, tabCount: 0, folderCount: 1 },
      { applicationId: "spotify", windowCount: 1, tabCount: 0, folderCount: 0 },
    ],
    repositories: [{ path: "C:\\work\\spider", branch: "retry-fix", staged: 1, modified: 3, untracked: 0 }],
  };
};

const settings: Settings = {
  capture: {
    includeWindows: true,
    excludedProcesses: [],
    excludedTitlePatterns: [],
    alwaysOn: ["spotify"],
    hiddenTitlePatterns: [],
  },
  browser: { captureTabs: true, excludedUrlPatterns: [], maxTabsPerWindow: 50 },
  retention: { keepLast: 20, maxAgeDays: null, autoPrune: false },
  timeouts: { gitMs: 5000, restoreOverallSecs: 120 },
  app: {
    notifications: true,
    closeToTray: true,
    startWithWindows: true,
    captureHotkey: "Ctrl+Alt+X",
    switcherHotkey: "Ctrl+Alt+W",
    autoSaveOnSwitch: true,
    offerResume: true,
    checkpointMinutes: 20,
    matchDisplaySetup: true,
  },
  logLevel: "info",
};

const runs: RestoreRunEntry[] = [
  { startedAt: ago(3), version: 3, overall: "success", steps: 5, problems: [] },
  {
    startedAt: ago(60 * 25),
    version: 2,
    overall: "partial_success",
    steps: 5,
    problems: [{ applicationId: "spotify", status: "failed", message: "its window did not appear within 30 seconds" }],
  },
];

const cleanup: CleanupConfig = {
  onEnter: [
    { process: "discord", mode: "graceful" },
    { process: "steam", mode: "force" },
  ],
  onExit: [],
};

const plan: CleanupPlan = {
  items: [
    {
      process: "discord",
      displayName: "Discord.exe",
      mode: "graceful",
      action: { kind: "close", processCount: 6, ramBytes: 412 * 1024 * 1024, cpuPercent: 1.2 },
      executables: ["C:\\Users\\me\\AppData\\Local\\Discord\\app-1.0.9\\Discord.exe"],
    },
    { process: "steam", displayName: "steam.exe", mode: "force", action: { kind: "not_running" }, executables: [] },
  ],
  ramBytes: 412 * 1024 * 1024,
  cpuPercent: 1.2,
};

const browser: BrowserStatus = {
  browsers: [
    { name: "Google Chrome", registered: true, connected: true, windows: 1, tabs: 7 },
    { name: "Microsoft Edge", registered: true, connected: false, windows: 0, tabs: 0 },
  ],
  manifestPresent: true,
  extensionUnpacked: true,
  extensionDir: "C:\\Users\\me\\AppData\\Roaming\\ContextFlow\\ContextFlow\\data\\browser\\extension",
};

const commands: WorkspaceCommandsDto = {
  onEnter: [{ program: "npm", args: ["run", "dev"], folder: "C:\\work\\spider", wait: false }],
  onLeave: [],
};

const info: AppInfo = {
  version: "0.1.0",
  databasePath: "C:\\Users\\me\\AppData\\Roaming\\ContextFlow\\data\\contextflow.db",
  configPath: "C:\\Users\\me\\AppData\\Roaming\\ContextFlow\\config\\config.toml",
  logsPath: "C:\\Users\\me\\AppData\\Roaming\\ContextFlow\\data\\logs",
};

const find = (id: unknown) => workspaces.find((w) => w.id === id || w.name === id) ?? workspaces[0]!;

/** Answers for the commands the screens read; changes are accepted and echoed. */
function answer(cmd: string, args: Record<string, unknown>): unknown {
  switch (cmd) {
    case "app_info":
      return info;
    case "list_workspaces":
      return workspaces.filter((w) => args.includeArchived || !w.isArchived);
    case "list_snapshots":
      return history[find(args.workspace).id] ?? [];
    case "get_snapshot": {
      const w = find(args.workspace);
      if (!history[w.id]?.length) throw { code: "not_found", message: "this workspace has no snapshots yet" };
      return detail(w.id, (args.version as number | null) ?? undefined);
    }
    case "list_restore_runs":
      return find(args.workspace).id === "5d0c6a2e-8b1f-4a37-9e2d-1f6b3c4a7e90" ? runs : [];
    case "get_cleanup_config":
      return find(args.workspace).id === "5d0c6a2e-8b1f-4a37-9e2d-1f6b3c4a7e90" ? cleanup : { onEnter: [], onExit: [] };
    case "set_cleanup_config":
      return args.config;
    case "preview_cleanup":
      return find(args.workspace).id === "5d0c6a2e-8b1f-4a37-9e2d-1f6b3c4a7e90" && args.phase === "entering" ? plan : { items: [], ramBytes: 0, cpuPercent: 0 };
    case "get_workspace_commands":
      return find(args.workspace).id === "5d0c6a2e-8b1f-4a37-9e2d-1f6b3c4a7e90" ? commands : { onEnter: [], onLeave: [] };
    case "set_workspace_commands":
      return args.commands;
    case "get_workspace_icon":
      return { custom: false, dataUrl: null };
    case "get_workspace_shortcuts":
      return [];
    case "get_settings":
    case "save_settings":
      return (args.settings as Settings | undefined) ?? settings;
    case "get_hotkey_status":
      return { capture: null, switcher: null };
    case "get_browser_status":
      return browser;
    case "health_check":
      return [
        { name: "Database", status: "ok", message: "healthy, 26 snapshots" },
        { name: "Browser extension", status: "warn", message: "Edge: not connected" },
      ];
    case "list_parked":
    case "search_workspaces":
      return [];
    case "list_timeline":
      return [
        { at: ago(3), kind: "switch", workspaceId: "5d0c6a2e-8b1f-4a37-9e2d-1f6b3c4a7e90", workspace: "Spider", fromId: "a93e17f4-2c5b-4d8e-b061-7c2f9e3d5a18", from: "Uni", version: 3, note: null },
        { at: ago(60 * 25), kind: "leave", workspaceId: "a93e17f4-2c5b-4d8e-b061-7c2f9e3d5a18", workspace: "Uni", fromId: null, from: null, version: 2, note: "Essay draft half done" },
      ];
    case "take_ui_action":
    case "take_resume":
      return null;
    case "set_save_mode":
      return { ...find(args.workspace), saveMode: args.mode };
    case "set_pinned_version":
      return { ...find(args.workspace), pinnedVersion: args.version };
    case "delete_old_snapshots":
      return [1, 2];
    case "restore_workspace":
    case "switch_workspace": {
      const request = args.request as { workspace: string; dryRun?: boolean };
      const w = find(request.workspace);
      const steps = detail(w.id).applications.map((app, i) => ({
        applicationId: app.applicationId,
        status: request.dryRun ? ("skipped" as const) : i === 4 ? ("failed" as const) : ("success" as const),
        code: request.dryRun ? "DRY_RUN" : i === 4 ? "WINDOW_TIMEOUT" : null,
        message: request.dryRun
          ? i % 2
            ? "would be started"
            : "already running; its windows would be repositioned"
          : i === 4
            ? "its window did not appear within 30 seconds"
            : "is back",
        durationMs: 300 + i * 120,
      }));
      return {
        workspace: w.name,
        workspaceId: w.id,
        version: history[w.id]?.[0]?.version ?? 1,
        dryRun: !!request.dryRun,
        cancelled: false,
        note: detail(w.id).notes[0] ?? null,
        overall: request.dryRun ? "skipped" : "partial_success",
        steps,
        cleanups: [],
        durationMs: 1650,
        savedAt: history[w.id]?.[0]?.createdAt ?? ago(5),
        repositories: [],
        switchedFrom: cmd === "switch_workspace" ? "Uni" : null,
        autoSavedVersion: cmd === "switch_workspace" ? 3 : null,
        autoSaveNote: null,
        layoutNote: null,
        commands: [],
      };
    }
    case "cancel_restore":
      return false;
    case "refresh_tray_menu":
    case "delete_snapshot":
      return null;
    default:
      console.warn(`[mock] no sample answer for "${cmd}"`, args);
      throw { code: "internal", message: `"${cmd}" isn't available in the preview` };
  }
}

export function installMockBackend() {
  mockWindows("main");
  mockIPC((cmd, payload) => answer(cmd, (payload ?? {}) as Record<string, unknown>), { shouldMockEvents: true });
  console.info("[mock] ContextFlow preview with sample data");
}
