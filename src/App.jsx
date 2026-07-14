import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { disable as disableAutostart, enable as enableAutostart, isEnabled as isAutostartEnabled } from "@tauri-apps/plugin-autostart";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowSync24Regular,
  Broom24Regular,
  Chat24Regular,
  CheckmarkCircle24Regular,
  ChevronDown20Regular,
  ChevronRight20Regular,
  Clock24Regular,
  DataTrending24Regular,
  Delete24Regular,
  Dismiss24Regular,
  Document24Regular,
  DocumentCopy24Regular,
  Folder24Regular,
  HardDrive24Regular,
  Home24Regular,
  MoreHorizontal24Regular,
  ScanCamera24Regular,
  Search24Regular,
  Settings24Regular,
  ShieldCheckmark24Regular,
  Sparkle24Regular,
  Warning24Regular,
} from "@fluentui/react-icons";
import lunaMark from "./assets/luna-clean.png";
import {
  DuplicatesView,
  LargeFilesView,
  OverviewView,
  ScanResultsView,
  ScheduleView,
  SettingsView,
  StorageView,
} from "./components/ScanViews.jsx";
import { formatBytes, formatDateTime, formatScanSize, getScanUsage } from "./lib/format.js";

const TrendsView = lazy(() => import("./components/TrendsView.jsx").then((module) => ({ default: module.TrendsView })));

const navigation = [
  { id: "overview", label: "Overview", icon: Home24Regular },
  { id: "scan", label: "Scan results", icon: ScanCamera24Regular },
  { id: "cleanup", label: "Cleanup review", icon: CheckmarkCircle24Regular },
  { id: "trends", label: "Trends", icon: DataTrending24Regular },
  { id: "storage", label: "Storage explorer", icon: Folder24Regular },
  { id: "duplicates", label: "Duplicates", icon: DocumentCopy24Regular },
  { id: "large", label: "Large files", icon: Document24Regular },
  { id: "schedule", label: "Schedule", icon: Clock24Regular },
  { id: "settings", label: "Settings", icon: Settings24Regular },
];

const initialItems = [
  {
    id: "browser-cache",
    group: "safe",
    name: "Browser cache",
    source: "Chrome, Edge, Firefox",
    size: 5.4,
    age: 126,
    date: "Mar 10, 2026",
    reason: "Temporary cache, images, and other files re-created as needed.",
    confidence: "High",
    icon: Search24Regular,
    detail:
      "This is safe cache data that browsers can rebuild. It doesn’t include bookmarks, passwords, browsing history, or settings.",
    examples: "Cached images, scripts, favicons, and service worker cache.",
    selected: true,
  },
  {
    id: "codex-cache",
    group: "safe",
    name: "Codex cache",
    source: "OpenAI Codex",
    size: 2.9,
    age: 74,
    date: "May 1, 2026",
    reason: "Build and response cache re-created when needed.",
    confidence: "High",
    icon: Broom24Regular,
    detail:
      "Only known cache and temporary paths are included. Threads, configuration, skills, and project files stay untouched.",
    examples: "Temporary downloads, generated cache entries, and disposable build state.",
    selected: true,
  },
  {
    id: "duplicate-installers",
    group: "review",
    name: "Duplicate installers",
    source: "Windows installers",
    size: 3.2,
    age: 182,
    date: "Jan 13, 2026",
    reason: "Installers for the same apps found in multiple locations.",
    confidence: "Medium",
    icon: DocumentCopy24Regular,
    detail:
      "These files share identical content hashes, but you may still want an offline installer. Luna leaves every copy unselected by default.",
    examples: "Repeated .exe and .msi downloads with matching SHA-256 hashes.",
    selected: false,
  },
  {
    id: "old-downloads",
    group: "review",
    name: "Downloads not opened in 90+ days",
    source: "Your Downloads folder",
    size: 7.1,
    age: 194,
    date: "Jan 1, 2026",
    reason: "Files not opened in 90+ days may no longer be needed.",
    confidence: "Low",
    icon: Delete24Regular,
    detail:
      "Age is a review signal, not proof that a file is disposable. Last-access timestamps can be incomplete on Windows, so Luna also considers modification dates.",
    examples: "Archives, media exports, and installers with no recent activity signal.",
    selected: false,
  },
];

const findings = [
  {
    title: "Caches are safe and rebuildable",
    body: "Browser and Codex caches total 8.3 GB. These are temporary files that apps will recreate as needed.",
    evidence: "5 sources",
  },
  {
    title: "Older files have low recent activity",
    body: "10.3 GB hasn’t been used in 90+ days. These are good candidates to review based on your activity.",
    evidence: "4 sources",
  },
  {
    title: "Most space comes from stale downloads",
    body: "7.1 GB is in Downloads not opened in 90+ days. Consider keeping only current projects.",
    evidence: "3 sources",
  },
];

function formatSize(value) {
  return `${value.toFixed(1)} GB`;
}

const cleanupIcons = {
  "browser-cache": Search24Regular,
  "codex-cache": Broom24Regular,
  "temp-files": Delete24Regular,
  "duplicate-files": DocumentCopy24Regular,
  "old-downloads": Delete24Regular,
};

function mapCleanupItem(item) {
  return {
    ...item,
    size: item.sizeBytes / 1024 ** 3,
    age: item.lastUsedDays ?? 0,
    date: item.lastUsedAt ? formatDateTime(item.lastUsedAt) : "Activity unknown",
    selected: item.selectedByDefault,
    icon: cleanupIcons[item.id] || Document24Regular,
  };
}

const previewAiReport = {
  model: "gpt-5.6-luna",
  generatedAt: "2026-07-14T09:24:00+02:00",
  responseId: "preview",
  report: {
    headline: "Caches are the clearest low-risk win",
    summary: "The aggregate scan points to rebuildable browser and Codex caches first. Older Downloads deserve review, but age alone is not evidence that a personal file should be removed.",
    riskLevel: "low",
    answer: "Start with the 8.3 GB of known caches. Then review older Downloads by project rather than deleting them as one group.",
    findings: [
      { title: "Rebuildable caches lead the plan", detail: "Browser and Codex caches account for 8.3 GB and can be recreated by their applications.", evidence: "2 safe categories · 8.3 GB", confidence: "high" },
      { title: "Older Downloads need judgment", detail: "The 90+ day signal identifies review candidates, not files that are proven disposable.", evidence: "7.1 GB · age metadata", confidence: "medium" },
      { title: "Exact copies may offer another win", detail: "Duplicate groups should be inspected so you can choose which location to keep.", evidence: "Content-hash groups", confidence: "medium" },
    ],
    actions: [
      { label: "Review safe caches", rationale: "Start with rebuildable data.", destination: "cleanup" },
      { label: "Inspect exact copies", rationale: "Choose the copy that belongs in your workflow.", destination: "duplicates" },
    ],
  },
};

function buildReportContext(scanResult, trendHistory, cleanupItems) {
  const latest = trendHistory?.snapshots?.at(-1);
  const totalBytes = scanResult ? getScanUsage(scanResult).usedBytes : latest?.totalBytes ?? Math.round(cleanupItems.reduce((sum, item) => sum + item.size, 0) * 1024 ** 3);
  const ageBuckets = scanResult?.ageBuckets || latest?.ageBuckets || {
    recentBytes: Math.round(0.6 * 1024 ** 3),
    inactive30To90Bytes: Math.round(2.1 * 1024 ** 3),
    inactive90To180Bytes: Math.round(3.4 * 1024 ** 3),
    inactive180PlusBytes: Math.round(10.3 * 1024 ** 3),
    unknownBytes: 0,
  };
  const categories = (scanResult?.categories || latest?.categories || cleanupItems).map((item) => ({
    name: item.name,
    sizeBytes: item.sizeBytes ?? Math.round((item.size || 0) * 1024 ** 3),
    fileCount: item.fileCount || 0,
    lastUsedDays: item.lastUsedDays ?? item.age ?? null,
  }));
  const cleanupSignals = (scanResult?.cleanupItems || cleanupItems).map((item) => ({
    name: item.name,
    group: item.group,
    sizeBytes: item.sizeBytes ?? Math.round((item.size || 0) * 1024 ** 3),
    fileCount: item.fileCount || 0,
    confidence: item.confidence,
  }));
  return {
    rootName: scanResult?.rootName || trendHistory?.rootName || "Selected storage",
    totalBytes,
    fileCount: scanResult?.fileCount ?? latest?.fileCount ?? 0,
    folderCount: scanResult?.folderCount ?? latest?.folderCount ?? 0,
    categories,
    cleanupSignals,
    ageBuckets,
    duplicateGroupCount: scanResult?.duplicateGroups?.length || 0,
    duplicateReclaimableBytes: scanResult?.duplicateGroups?.reduce((sum, group) => sum + group.reclaimableBytes, 0) ?? latest?.duplicateReclaimableBytes ?? 0,
    trendSnapshots: (trendHistory?.snapshots || []).map((snapshot) => ({
      capturedAt: snapshot.capturedAt,
      totalBytes: snapshot.totalBytes,
      inactive180PlusBytes: snapshot.ageBuckets.inactive180PlusBytes,
    })),
  };
}

function NavItem({ item, active, onClick }) {
  const Icon = item.icon;
  return (
    <button
      className={`nav-item ${active ? "is-active" : ""}`}
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
    >
      <Icon aria-hidden="true" />
      <span>{item.label}</span>
    </button>
  );
}

function CleanupRow({ item, expanded, onExpand, onSelect }) {
  const Icon = item.icon;
  const review = item.group === "review";
  return (
    <div className={`cleanup-row-wrap ${expanded ? "is-expanded" : ""}`}>
      <div className="cleanup-row">
        <label className="check-cell">
          <input
            type="checkbox"
            checked={item.selected}
            onChange={() => onSelect(item.id)}
            aria-label={`Select ${item.name}`}
          />
        </label>
        <div className={`item-icon ${review ? "is-review" : ""}`}>
          <Icon aria-hidden="true" />
        </div>
        <div className="item-name">
          <strong>{item.name}</strong>
          <span>{item.source}</span>
        </div>
        <strong className="item-size">{formatSize(item.size)}</strong>
        <div className="item-age">
          <span>Last used</span>
          <strong>{item.age} days ago</strong>
          <span>{item.date}</span>
        </div>
        <p className="item-reason">{item.reason}</p>
        <span className={`confidence confidence-${item.confidence.toLowerCase()}`}>
          {item.confidence}
        </span>
        <button
          className="icon-button row-expand"
          type="button"
          onClick={() => onExpand(item.id)}
          aria-label={`${expanded ? "Collapse" : "Expand"} evidence for ${item.name}`}
          aria-expanded={expanded}
        >
          {expanded ? <ChevronDown20Regular /> : <ChevronRight20Regular />}
        </button>
      </div>
      {expanded && (
        <div className="evidence-row">
          <span className={`evidence-accent ${review ? "is-review" : ""}`} />
          <div>
            <p>{item.detail}</p>
            <span>Examples: {item.examples}</span>
          </div>
          <button className="evidence-link" type="button">
            <Document24Regular aria-hidden="true" />
            3 sources
            <ChevronRight20Regular aria-hidden="true" />
          </button>
        </div>
      )}
    </div>
  );
}

function CleanupGroup({ title, description, kind, items, expandedId, onExpand, onSelect }) {
  const selected = items.filter((item) => item.selected).length;
  const total = items.reduce((sum, item) => sum + item.size, 0);
  const safe = kind === "safe";
  return (
    <section className="cleanup-group">
      <div className="group-heading">
        <div className={`group-symbol ${safe ? "is-safe" : "is-review"}`}>
          {safe ? <CheckmarkCircle24Regular /> : <Warning24Regular />}
        </div>
        <div>
          <h2>
            {title} <span>({formatSize(total)})</span>
          </h2>
          <p>{description}</p>
        </div>
        <span className="selection-count">
          {selected} of {items.length} selected
        </span>
        <ChevronDown20Regular aria-hidden="true" />
      </div>
      <div className="column-labels" aria-hidden="true">
        <span>Item</span>
        <span>Size</span>
        <span>Last used</span>
        <span>{safe ? "Why safe to remove" : "Why review"}</span>
        <span>Confidence</span>
      </div>
      {items.map((item) => (
        <CleanupRow
          key={item.id}
          item={item}
          expanded={expandedId === item.id}
          onExpand={onExpand}
          onSelect={onSelect}
        />
      ))}
    </section>
  );
}

function ConfirmDialog({ count, size, busy, onCancel, onConfirm }) {
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onCancel}>
      <section
        className="confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <button className="icon-button dialog-close" type="button" onClick={onCancel}>
          <Dismiss24Regular />
          <span className="sr-only">Close</span>
        </button>
        <span className="dialog-icon"><ShieldCheckmark24Regular /></span>
        <h2 id="confirm-title">Ready to reclaim {formatSize(size)}?</h2>
        <p>
          Luna will remove {count} selected {count === 1 ? "item" : "items"}. Review-only
          files stay untouched unless you selected them yourself.
        </p>
        <div className="dialog-note">
          <CheckmarkCircle24Regular />
          Known cache paths only. Personal settings are excluded.
        </div>
        <div className="dialog-actions">
          <button className="secondary-button" type="button" onClick={onCancel} disabled={busy}>Keep reviewing</button>
          <button className="primary-button" type="button" onClick={onConfirm} disabled={busy}>{busy ? "Cleaning safely…" : "Clean selected files"}</button>
        </div>
      </section>
    </div>
  );
}

export function App() {
  const isTauri = Boolean(window.__TAURI_INTERNALS__);
  const [items, setItems] = useState(initialItems);
  const [expandedId, setExpandedId] = useState("browser-cache");
  const [activeNav, setActiveNav] = useState("cleanup");
  const [confirming, setConfirming] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [toast, setToast] = useState("");
  const [followUpOpen, setFollowUpOpen] = useState(false);
  const [question, setQuestion] = useState("");
  const [appVersion, setAppVersion] = useState("0.6.3");
  const [scanRoots, setScanRoots] = useState([]);
  const [selectedRoot, setSelectedRoot] = useState("");
  const [scanResult, setScanResult] = useState(null);
  const [scanProgress, setScanProgress] = useState(null);
  const [scanError, setScanError] = useState("");
  const [scanning, setScanning] = useState(false);
  const [trendHistory, setTrendHistory] = useState(null);
  const [scheduleStatus, setScheduleStatus] = useState({ enabled: false, frequency: "Weekly", scanRoot: "", isScanning: false });
  const [startupEnabled, setStartupEnabled] = useState(false);
  const [startupBusy, setStartupBusy] = useState(false);
  const [aiStatus, setAiStatus] = useState({ configured: false, model: "gpt-5.6-luna", source: "none" });
  const [aiReport, setAiReport] = useState(null);
  const [aiBusy, setAiBusy] = useState(false);
  const [updateState, setUpdateState] = useState({ phase: "idle", currentVersion: "0.6.3", availableVersion: "", progress: 0, message: "" });
  const updateRef = useRef(null);

  const selectedItems = useMemo(() => items.filter((item) => item.selected), [items]);
  const selectedSize = useMemo(
    () => selectedItems.reduce((sum, item) => sum + item.size, 0),
    [selectedItems],
  );
  const cleanupTotal = useMemo(
    () => items.reduce((sum, item) => sum + item.size, 0),
    [items],
  );
  const currentRoot = useMemo(
    () => scanRoots.find((root) => root.path === selectedRoot) || scanRoots.find((root) => root.kind !== "home"),
    [scanRoots, selectedRoot],
  );

  useEffect(() => {
    if (!isTauri) return;
    setTrendHistory(null);
    Promise.all([invoke("app_status"), invoke("list_scan_roots"), invoke("get_schedule_status"), isAutostartEnabled(), invoke("ai_status")])
      .then(([status, roots, schedule, autostart, ai]) => {
        setAppVersion(status.version);
        setScanRoots(roots);
        setSelectedRoot(status.defaultScanRoot || roots[0]?.path || "");
        setScheduleStatus(schedule);
        setStartupEnabled(autostart);
        setAiStatus(ai);
        setUpdateState((current) => ({ ...current, currentVersion: status.version }));
      })
      .catch(() => undefined);
  }, [isTauri]);

  useEffect(() => {
    if (!isTauri) return undefined;
    const timer = window.setTimeout(() => checkForUpdates(true), 1200);
    return () => window.clearTimeout(timer);
  }, [isTauri]);

  useEffect(() => {
    if (!isTauri || !selectedRoot) return;
    invoke("get_trend_history", { root: selectedRoot })
      .then(setTrendHistory)
      .catch(() => setTrendHistory(null));
  }, [isTauri, selectedRoot]);

  useEffect(() => {
    if (!isTauri) return undefined;
    let stopListening = [];
    let disposed = false;
    Promise.all([
      listen("scan-progress", (event) => setScanProgress(event.payload)),
      listen("scheduled-scan-started", () => {
        setScheduleStatus((current) => ({ ...current, isScanning: true, lastError: null }));
        setToast("Luna is capturing a storage snapshot quietly in the background.");
      }),
      listen("scheduled-scan-complete", async (event) => {
        setScheduleStatus((current) => ({ ...current, isScanning: false, lastRunAt: event.payload.scannedAt }));
        setToast(`Snapshot captured — ${formatBytes(event.payload.totalBytes)} measured.`);
        invoke("get_schedule_status").then(setScheduleStatus).catch(() => undefined);
        if (event.payload.root === selectedRoot) {
          invoke("get_trend_history", { root: selectedRoot }).then(setTrendHistory).catch(() => undefined);
        }
      }),
      listen("scheduled-scan-error", (event) => {
        setScheduleStatus((current) => ({ ...current, isScanning: false, lastError: String(event.payload) }));
        setToast(`Scheduled snapshot stopped: ${String(event.payload)}`);
      }),
    ]).then((unlisteners) => {
      if (disposed) {
        unlisteners.forEach((unlisten) => unlisten());
      } else {
        stopListening = unlisteners;
      }
    });
    return () => {
      disposed = true;
      stopListening.forEach((unlisten) => unlisten());
    };
  }, [isTauri, selectedRoot]);

  useEffect(() => {
    if (!toast) return undefined;
    const timer = window.setTimeout(() => setToast(""), 4200);
    return () => window.clearTimeout(timer);
  }, [toast]);

  function toggleItem(id) {
    setItems((current) =>
      current.map((item) => (item.id === id ? { ...item, selected: !item.selected } : item)),
    );
  }

  async function runScan(path = selectedRoot) {
    if (!isTauri) {
      setToast("Run Luna Clean with “npm run tauri dev” to scan the local file system.");
      return;
    }
    if (!path) {
      setScanError("Choose a folder or drive first.");
      return;
    }
    setScanning(true);
    setScanError("");
    setScanProgress({ scannedFiles: 0, scannedBytes: 0, currentPath: path });
    try {
      const result = await invoke("scan_path", { path });
      setScanResult(result);
      setSelectedRoot(path);
      setItems(result.cleanupItems.map(mapCleanupItem));
      const history = await invoke("get_trend_history", { root: path });
      setTrendHistory(history);
      setAiReport(null);
      setExpandedId(result.cleanupItems.find((item) => item.sizeBytes > 0)?.id || "");
      setToast(`Scan complete — ${formatScanSize(result)} across ${result.fileCount.toLocaleString()} files.`);
    } catch (error) {
      setScanError(String(error));
      setToast(`Scan stopped: ${String(error)}`);
    } finally {
      setScanning(false);
    }
  }

  async function chooseScanFolder() {
    if (!isTauri) {
      setToast("Folder selection is available in the native Tauri app.");
      return;
    }
    const selection = await open({
      directory: true,
      multiple: false,
      title: "Choose a folder or drive to scan",
      defaultPath: selectedRoot || undefined,
    });
    if (typeof selection === "string") {
      setSelectedRoot(selection);
      await runScan(selection);
    }
  }

  async function changeSchedule(next) {
    if (!isTauri) {
      setScheduleStatus((current) => ({ ...current, ...next }));
      setToast("Native scheduling is available in the Tauri app.");
      return;
    }
    try {
      const updated = await invoke("update_schedule", { request: next });
      setScheduleStatus(updated);
      setToast(updated.enabled ? `${updated.frequency} snapshots are scheduled.` : "Scheduled snapshots are off.");
    } catch (error) {
      setToast(`Schedule unchanged: ${String(error)}`);
    }
  }

  async function captureScheduledSnapshot() {
    if (!isTauri) {
      setToast("Background snapshots are available in the native Tauri app.");
      return;
    }
    try {
      await invoke("capture_scheduled_snapshot");
      setScheduleStatus((current) => ({ ...current, isScanning: true }));
    } catch (error) {
      setToast(String(error));
    }
  }

  async function toggleStartup() {
    if (!isTauri || startupBusy) return;
    setStartupBusy(true);
    try {
      if (startupEnabled) {
        await disableAutostart();
      } else {
        await enableAutostart();
      }
      const enabled = await isAutostartEnabled();
      setStartupEnabled(enabled);
      setToast(enabled ? "Luna will start quietly in the tray with Windows." : "Windows startup is off.");
    } catch (error) {
      setToast(`Startup setting unchanged: ${String(error)}`);
    } finally {
      setStartupBusy(false);
    }
  }

  async function saveApiKey(apiKey) {
    if (!isTauri) {
      setAiStatus({ configured: true, model: "gpt-5.6-luna", source: "windowsCredentialManager" });
      setToast("Preview key validated and stored securely.");
      return;
    }
    const updated = await invoke("save_api_key", { request: { apiKey } });
    setAiStatus(updated);
    setToast("OpenAI key validated and saved in Windows Credential Manager.");
  }

  async function removeApiKey() {
    if (!isTauri) {
      const updated = { configured: false, model: "gpt-5.6-luna", source: "none" };
      setAiStatus(updated);
      setToast("Preview saved key removed.");
      return updated;
    }
    const updated = await invoke("delete_api_key");
    setAiStatus(updated);
    setToast(updated.configured
      ? "Saved key removed. Luna is using the development environment fallback."
      : "Saved OpenAI key removed from Windows Credential Manager.");
    return updated;
  }

  async function checkForUpdates(silent = false) {
    if (!isTauri) {
      setUpdateState((current) => ({ ...current, phase: "current", message: "Update checks run in the installed Windows app." }));
      return;
    }
    setUpdateState((current) => ({ ...current, phase: "checking", progress: 0, message: "Checking the signed release channel…" }));
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();
      updateRef.current = update;
      if (update) {
        setUpdateState((current) => ({
          ...current,
          phase: "available",
          availableVersion: update.version,
          message: `Luna Clean ${update.version} is ready to install.`,
        }));
        if (!silent) setToast(`Luna Clean ${update.version} is available.`);
      } else {
        setUpdateState((current) => ({ ...current, phase: "current", availableVersion: "", message: "You have the newest signed release." }));
        if (!silent) setToast("Luna Clean is up to date.");
      }
    } catch (error) {
      updateRef.current = null;
      setUpdateState((current) => ({ ...current, phase: "error", message: String(error) }));
      if (!silent) setToast(`Update check stopped: ${String(error)}`);
    }
  }

  async function installUpdate() {
    const update = updateRef.current;
    if (!update) {
      await checkForUpdates();
      return;
    }
    let downloaded = 0;
    let contentLength = 0;
    setUpdateState((current) => ({ ...current, phase: "downloading", progress: 0, message: "Downloading the signed installer…" }));
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          contentLength = event.data.contentLength || 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          const progress = contentLength ? Math.min(Math.round((downloaded / contentLength) * 100), 99) : 0;
          setUpdateState((current) => ({ ...current, phase: "downloading", progress, message: progress ? `Downloading update · ${progress}%` : "Downloading the signed installer…" }));
        } else if (event.event === "Finished") {
          setUpdateState((current) => ({ ...current, phase: "installing", progress: 100, message: "Installing securely…" }));
        }
      });
      setUpdateState((current) => ({ ...current, phase: "restarting", progress: 100, message: "Restarting into the new version…" }));
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (error) {
      setUpdateState((current) => ({ ...current, phase: "error", message: String(error) }));
      setToast(`Update installation stopped: ${String(error)}`);
    }
  }

  async function completeCleanup() {
    const count = selectedItems.length;
    const size = selectedSize;
    if (!isTauri) {
      setConfirming(false);
      setItems((current) => current.map((item) => ({ ...item, selected: false })));
      setToast(`Cleanup preview complete — ${formatSize(size)} across ${count} items.`);
      return;
    }
    setCleaning(true);
    try {
      const result = await invoke("clean_items", {
        request: { itemIds: selectedItems.map((item) => item.id) },
      });
      setItems((current) => current.map((item) => ({ ...item, selected: false })));
      const skipped = result.skipped.length ? ` ${result.skipped.length} review item(s) stayed untouched.` : "";
      setToast(`Removed ${formatBytes(result.removedBytes)} from ${result.removedFiles.toLocaleString()} files.${skipped}`);
      setConfirming(false);
    } catch (error) {
      setToast(`Cleanup stopped safely: ${String(error)}`);
    } finally {
      setCleaning(false);
    }
  }

  async function runAiInvestigation(userQuestion = "") {
    if (!isTauri) {
      setAiReport(previewAiReport);
      setToast("Preview report generated from the sample aggregate scan.");
      return;
    }
    if (!aiStatus.configured) {
      setActiveNav("settings");
      setToast("Add an OpenAI API key in Settings to enable Luna reports.");
      return;
    }
    if (!scanResult && !trendHistory?.snapshots?.length) {
      setToast("Run a scan first so Luna has local aggregate evidence to investigate.");
      return;
    }
    setAiBusy(true);
    try {
      const envelope = await invoke("generate_ai_report", {
        request: {
          question: userQuestion.trim() || null,
          context: buildReportContext(scanResult, trendHistory, items),
        },
      });
      setAiReport(envelope);
      setToast(`Luna’s report is ready · ${envelope.model}`);
    } catch (error) {
      setToast(`Luna report stopped: ${String(error)}`);
    } finally {
      setAiBusy(false);
    }
  }

  async function submitQuestion(event) {
    event.preventDefault();
    if (!question.trim()) return;
    const submitted = question;
    setQuestion("");
    setFollowUpOpen(false);
    await runAiInvestigation(submitted);
  }

  const safeItems = items.filter((item) => item.group === "safe");
  const reviewItems = items.filter((item) => item.group === "review");
  const viewProps = {
    result: scanResult,
    scanning,
    progress: scanProgress,
    error: scanError,
    onScan: () => runScan(),
    onChooseFolder: chooseScanFolder,
  };
  const featureViews = {
    overview: <OverviewView {...viewProps} />,
    scan: <ScanResultsView {...viewProps} />,
    storage: <StorageView {...viewProps} />,
    duplicates: <DuplicatesView {...viewProps} />,
    large: <LargeFilesView {...viewProps} />,
    schedule: (
      <ScheduleView
        schedule={scheduleStatus}
        roots={scanRoots}
        selectedRoot={selectedRoot}
        onScheduleChange={changeSchedule}
        onCapture={captureScheduledSnapshot}
      />
    ),
    settings: (
      <SettingsView
        roots={scanRoots}
        selectedRoot={selectedRoot}
        onRootChange={setSelectedRoot}
        onScan={() => runScan()}
        onChooseFolder={chooseScanFolder}
        startupEnabled={startupEnabled}
        startupBusy={startupBusy}
        onStartupToggle={toggleStartup}
        aiStatus={aiStatus}
        onSaveApiKey={saveApiKey}
        onRemoveApiKey={removeApiKey}
        updateState={updateState}
        onCheckForUpdates={() => checkForUpdates(false)}
        onInstallUpdate={installUpdate}
      />
    ),
  };
  const usedPercent = currentRoot?.totalBytes
    ? ((currentRoot.totalBytes - currentRoot.availableBytes) / currentRoot.totalBytes) * 100
    : 62;
  const liveFindings = aiReport
    ? aiReport.report.findings.map((finding) => ({
        title: finding.title,
        body: finding.detail,
        evidence: `${finding.evidence} · ${finding.confidence} confidence`,
      }))
    : scanResult
      ? [
        {
          title: "Caches are isolated from personal data",
          body: `${formatBytes(safeItems.reduce((sum, item) => sum + (item.sizeBytes || 0), 0))} sits in known rebuildable cache paths.`,
          evidence: `${safeItems.reduce((sum, item) => sum + (item.evidenceCount || 0), 0)} sources`,
        },
        {
          title: "Older files have low recent activity",
          body: `${formatBytes(scanResult.ageBuckets.inactive90To180Bytes + scanResult.ageBuckets.inactive180PlusBytes)} has no activity signal in 90+ days.`,
          evidence: "scan metadata",
        },
        {
          title: "Exact copies are ready for review",
          body: `${scanResult.duplicateGroups.length} content-hash groups can be inspected without assuming which copy should be kept.`,
          evidence: "BLAKE3 hashes",
        },
        ]
      : findings;
  const findingsIntro = aiReport
    ? aiReport.report.summary
    : scanResult
      ? "I analyzed your local scan metadata. Ask Luna for a GPT-5.6 investigation when you want a deeper report."
      : "I analyzed the preview scan results and here’s what stands out.";
  const ageChartEntries = scanResult
    ? [
        [formatBytes(scanResult.ageBuckets.recentBytes), "0–30", "days", scanResult.ageBuckets.recentBytes, "mint"],
        [formatBytes(scanResult.ageBuckets.inactive30To90Bytes), "31–90", "days", scanResult.ageBuckets.inactive30To90Bytes, "green"],
        [formatBytes(scanResult.ageBuckets.inactive90To180Bytes), "91–180", "days", scanResult.ageBuckets.inactive90To180Bytes, "amber"],
        [formatBytes(scanResult.ageBuckets.inactive180PlusBytes), "180+", "days", scanResult.ageBuckets.inactive180PlusBytes, "orange"],
      ]
    : [
        ["0.6 GB", "0–30", "days", 14, "mint"],
        ["2.1 GB", "31–90", "days", 28, "green"],
        ["3.4 GB", "91–180", "days", 42, "amber"],
        ["10.3 GB", "180+", "days", 82, "orange"],
      ];
  const ageMaximum = scanResult ? Math.max(...ageChartEntries.map((entry) => entry[3]), 1) : 100;

  return (
    <div className={`app-shell ${!["cleanup", "trends"].includes(activeNav) ? "without-findings" : ""}`}>
      <aside className="sidebar">
        <div className="brand-lockup">
          <img src={lunaMark} alt="" />
          <span>Luna Clean</span>
          <small>PRO</small>
        </div>
        <button className="icon-button menu-button" type="button" aria-label="Collapse navigation">
          <MoreHorizontal24Regular />
        </button>
        <nav aria-label="Primary navigation">
          {navigation.map((item) => (
            <NavItem
              key={item.id}
              item={item}
              active={activeNav === item.id}
              onClick={() => setActiveNav(item.id)}
            />
          ))}
        </nav>
        <div className="sidebar-spacer" />
        <div className="drive-summary">
          <div>
            <HardDrive24Regular />
            <span>{currentRoot?.name || "Local Disk (C:)"}</span>
          </div>
          <small>{currentRoot?.totalBytes ? `${formatBytes(currentRoot.availableBytes)} free of ${formatBytes(currentRoot.totalBytes)}` : "Choose a drive to measure capacity"}</small>
          <progress value={usedPercent} max="100">{Math.round(usedPercent)}%</progress>
        </div>
        <button className="rescan-button" type="button" disabled={scanning} onClick={() => runScan()}>
          <ArrowSync24Regular />
          {scanning ? "Scanning" : "Rescan"}
        </button>
        <span className="scan-stamp">{scanResult ? `Scanned ${formatDateTime(scanResult.scannedAt)}` : "Ready for a local scan"}<br />v{appVersion}</span>
      </aside>

      {activeNav === "trends" ? (
        <Suspense fallback={<main className="trends-workspace trend-empty-workspace"><div className="trend-loading">Drawing your storage story…</div></main>}>
          <TrendsView
            history={trendHistory}
            onCapture={() => runScan()}
            onAsk={() => runAiInvestigation("Investigate the storage trend over time. Explain the most important movement and the safest next step.")}
            aiReport={aiReport}
            aiBusy={aiBusy}
            scanning={scanning}
            progress={scanProgress}
          />
        </Suspense>
      ) : activeNav !== "cleanup" ? (
        <main className="feature-workspace">{featureViews[activeNav]}</main>
      ) : (
      <>
      <main className="review-workspace">
        <header className="review-header">
          <div>
            <h1>A careful plan to review {formatSize(cleanupTotal)}</h1>
            <p>We’ve analyzed what you don’t need. Review with confidence—nothing is deleted without confirmation.</p>
            <div className="scan-trust">
              <ShieldCheckmark24Regular />
              <span>Based on scan results only</span>
              <i />
              <time dateTime={scanResult?.scannedAt || "2026-07-14T09:18:00"}>{scanResult ? formatDateTime(scanResult.scannedAt) : "Jul 14, 2026  09:18 AM"}</time>
            </div>
          </div>
          <div className="header-action">
            <button
              className="primary-button clean-button"
              type="button"
              disabled={!selectedItems.length}
              onClick={() => setConfirming(true)}
            >
              Clean selected files ({formatSize(selectedSize)})
            </button>
            <span><ShieldCheckmark24Regular /> Nothing is deleted without confirmation</span>
          </div>
        </header>

        <CleanupGroup
          title="Safe to remove"
          description="High confidence. Temporary or cache files you can safely remove."
          kind="safe"
          items={safeItems}
          expandedId={expandedId}
          onExpand={(id) => setExpandedId((current) => (current === id ? "" : id))}
          onSelect={toggleItem}
        />
        <CleanupGroup
          title="Needs your review"
          description="Review recommended. These files may be useful."
          kind="review"
          items={reviewItems}
          expandedId={expandedId}
          onExpand={(id) => setExpandedId((current) => (current === id ? "" : id))}
          onSelect={toggleItem}
        />

        <footer className="control-note">
          <span><CheckmarkCircle24Regular /></span>
          <p>You’re in control. Items in “Needs your review” are not selected by default.</p>
          <button type="button" onClick={() => setToast("Review-sensitive items stay unselected unless you choose them.")}>
            Review selections <ChevronRight20Regular />
          </button>
        </footer>
      </main>

      <aside className="findings-panel">
        <div className="findings-heading">
          <Sparkle24Regular />
          <h2>Luna’s findings</h2>
          <button className="icon-button" type="button" aria-label="More report actions"><MoreHorizontal24Regular /></button>
        </div>
        <p className="findings-intro">{findingsIntro}</p>
        <button className="report-trigger" type="button" disabled={aiBusy} onClick={() => runAiInvestigation()}>
          <Sparkle24Regular /> {aiBusy ? "Luna is investigating…" : aiReport ? "Refresh GPT-5.6 report" : "Investigate with GPT-5.6-Luna"}
        </button>
        {aiReport?.report.answer && <p className="ai-answer">{aiReport.report.answer}</p>}
        <ol className="findings-list">
          {liveFindings.map((finding, index) => (
            <li key={finding.title}>
              <span className="finding-number">{index + 1}</span>
              <div>
                <h3>{finding.title}</h3>
                <p>{finding.body}</p>
                <button type="button">
                  <Document24Regular /> Evidence: {finding.evidence} <ChevronRight20Regular />
                </button>
              </div>
            </li>
          ))}
        </ol>
        <div className="age-chart">
          <h3>Age distribution <span>(all reviewed items)</span></h3>
          <div className="chart-body" aria-label="Age distribution: 0.6 GB zero to thirty days, 2.1 GB thirty-one to ninety days, 3.4 GB ninety-one to one-hundred-eighty days, 10.3 GB over one-hundred-eighty days">
            {ageChartEntries.map(([value, label, unit, rawHeight, color]) => (
              <div className="chart-column" key={label}>
                <span>{value}</span>
                <i className={`bar bar-${color}`} style={{ height: `${Math.max((rawHeight / ageMaximum) * 82, rawHeight ? 5 : 0)}%` }} />
                <strong>{label}</strong>
                <small>{unit}</small>
              </div>
            ))}
          </div>
        </div>
        <div className="follow-up-area">
          {followUpOpen ? (
            <form onSubmit={submitQuestion}>
              <label htmlFor="luna-question">Ask about this plan</label>
              <textarea
                id="luna-question"
                value={question}
                onChange={(event) => setQuestion(event.target.value)}
                placeholder="Why is this safe to remove?"
                autoFocus
              />
              <div>
                <button className="secondary-button" type="button" onClick={() => setFollowUpOpen(false)}>Cancel</button>
                <button className="primary-button" type="submit" disabled={aiBusy}>{aiBusy ? "Investigating…" : "Ask Luna"}</button>
              </div>
            </form>
          ) : (
            <button className="follow-up-button" type="button" onClick={() => setFollowUpOpen(true)}>
              <Chat24Regular /> Ask a follow-up
            </button>
          )}
          <span>{aiReport ? `${aiReport.model} · report generated ${formatDateTime(aiReport.generatedAt)}` : `${aiStatus.model} · aggregate metadata only`}</span>
        </div>
      </aside>
      </>
      )}

      {confirming && (
        <ConfirmDialog
          count={selectedItems.length}
          size={selectedSize}
          busy={cleaning}
          onCancel={() => setConfirming(false)}
          onConfirm={completeCleanup}
        />
      )}
      {toast && <div className="toast" role="status">{toast}</div>}
    </div>
  );
}
