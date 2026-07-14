import { useMemo, useState } from "react";
import {
  ArrowRight24Regular,
  ArrowDownload24Regular,
  ArrowSync24Regular,
  CalendarClock24Regular,
  CheckmarkCircle24Regular,
  DocumentCopy24Regular,
  Folder24Regular,
  HardDrive24Regular,
  Info24Regular,
  LockClosed24Regular,
  Search24Regular,
  Settings24Regular,
  ShieldCheckmark24Regular,
  Sparkle24Regular,
  Warning24Regular,
} from "@fluentui/react-icons";
import { formatBytes, formatCount, formatDateTime, formatDuration, formatScanProgressSize, formatScanSize, getScanUsage } from "../lib/format.js";

function EmptyScan({ onScan, onChooseFolder }) {
  return (
    <section className="empty-scan">
      <span><Search24Regular /></span>
      <h2>See where your space went</h2>
      <p>Choose a folder or drive. Luna scans file metadata locally and keeps file contents on this PC.</p>
      <div>
        <button className="primary-button" type="button" onClick={onScan}>Scan default folder</button>
        <button className="secondary-button" type="button" onClick={onChooseFolder}>Choose a folder</button>
      </div>
    </section>
  );
}

function FeatureHeader({ eyebrow, title, description, action }) {
  return (
    <header className="feature-header">
      <div>
        <span>{eyebrow}</span>
        <h1>{title}</h1>
        <p>{description}</p>
      </div>
      {action}
    </header>
  );
}

function ScanProgress({ progress }) {
  return (
    <section className="scan-progress-panel" aria-live="polite">
      <span className="scan-spinner"><ArrowSync24Regular /></span>
      <div>
        <h2>Reading storage metadata…</h2>
        <p>{progress?.currentPath || "Preparing the scan"}</p>
        <span>{formatCount(progress?.scannedFiles || 0)} files · {formatScanProgressSize(progress)}</span>
      </div>
      <div className="indeterminate-track"><i /></div>
    </section>
  );
}

function AgeDistribution({ result }) {
  const entries = [
    ["0–30 days", result.ageBuckets.recentBytes, "fresh"],
    ["31–90 days", result.ageBuckets.inactive30To90Bytes, "warm"],
    ["91–180 days", result.ageBuckets.inactive90To180Bytes, "stale"],
    ["180+ days", result.ageBuckets.inactive180PlusBytes, "old"],
  ];
  const maximum = Math.max(...entries.map((entry) => entry[1]), 1);
  return (
    <section className="feature-surface age-distribution">
      <div className="surface-heading">
        <div><h2>Activity age</h2><p>Last-access or modification signal for scanned files.</p></div>
      </div>
      <div className="age-rows">
        {entries.map(([label, bytes, tone]) => (
          <div className="age-row" key={label}>
            <span>{label}</span>
            <div><i className={`tone-${tone}`} style={{ width: `${Math.max((bytes / maximum) * 100, bytes ? 2 : 0)}%` }} /></div>
            <strong>{formatBytes(bytes)}</strong>
          </div>
        ))}
      </div>
      <p className="timestamp-caveat"><Info24Regular /> Windows may disable or defer last-access updates. Luna uses modification time as a fallback and treats age as review evidence, not a deletion rule.</p>
    </section>
  );
}

function CategoryTable({ result, limit }) {
  const maximum = Math.max(...result.categories.map((category) => category.sizeBytes), 1);
  return (
    <section className="feature-surface category-table">
      <div className="surface-heading">
        <div><h2>Largest areas</h2><p>Grouped by the first folder beneath the selected root.</p></div>
        <span>{result.categories.length} areas</span>
      </div>
      <div className="data-table category-data-table">
        <div className="data-header"><span>Name</span><span>Space</span><span>Files</span><span>Last activity</span></div>
        {result.categories.slice(0, limit).map((category) => (
          <div className="data-row" key={category.path}>
            <div className="path-cell"><Folder24Regular /><span><strong>{category.name}</strong><small>{category.path}</small></span></div>
            <div className="size-with-bar"><strong>{formatBytes(category.sizeBytes)}</strong><i><b style={{ width: `${Math.max((category.sizeBytes / maximum) * 100, 2)}%` }} /></i></div>
            <span>{formatCount(category.fileCount)}</span>
            <span>{category.lastUsedDays == null ? "Unknown" : `${category.lastUsedDays} days ago`}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

export function OverviewView({ result, scanning, progress, onScan, onChooseFolder }) {
  if (scanning) return <ScanProgress progress={progress} />;
  if (!result) return <EmptyScan onScan={onScan} onChooseFolder={onChooseFolder} />;
  const reclaimable = result.cleanupItems.reduce((sum, item) => sum + item.sizeBytes, 0);
  const scanUsage = getScanUsage(result);
  return (
    <div className="feature-view">
      <FeatureHeader
        eyebrow="Overview"
        title={`A clear view of ${result.rootName}`}
        description={`Scanned ${formatCount(result.fileCount)} files without uploading file contents.`}
        action={<button className="primary-button" type="button" onClick={onScan}><ArrowSync24Regular /> Scan again</button>}
      />
      <section className="summary-strip">
        <div><span>{scanUsage.isDrive ? "Windows drive usage" : "Scanned data"}</span><strong>{formatBytes(scanUsage.usedBytes)}</strong><small>{scanUsage.isDrive ? `${formatBytes(scanUsage.totalBytes)} total` : `${formatCount(result.folderCount)} folders`}</small></div>
        <div><span>Potential review</span><strong>{formatBytes(reclaimable)}</strong><small>{result.cleanupItems.length} cleanup signals</small></div>
        <div><span>Exact duplicates</span><strong>{result.duplicateGroups.length}</strong><small>Content-hash groups</small></div>
        <div><span>Scan time</span><strong>{formatDuration(result.durationMs)}</strong><small>{formatDateTime(result.scannedAt)}</small></div>
      </section>
      <CategoryTable result={result} limit={8} />
      <AgeDistribution result={result} />
    </div>
  );
}

export function ScanResultsView({ result, scanning, progress, error, onScan, onChooseFolder }) {
  if (scanning) return <ScanProgress progress={progress} />;
  return (
    <div className="feature-view">
      <FeatureHeader
        eyebrow="Scan results"
        title={result ? "The scan is ready to review" : "Start a local storage scan"}
        description={result ? result.root : "Luna reads names, sizes, and timestamps locally. It does not open file contents during a storage scan."}
        action={<div className="feature-actions"><button className="secondary-button" type="button" onClick={onChooseFolder}>Choose folder</button><button className="primary-button" type="button" onClick={onScan}>Scan now</button></div>}
      />
      {error && <div className="inline-error"><Warning24Regular />{error}</div>}
      {!result ? <EmptyScan onScan={onScan} onChooseFolder={onChooseFolder} /> : (
        <>
          <section className="scan-summary-line"><CheckmarkCircle24Regular /><div><strong>Completed in {formatDuration(result.durationMs)}</strong><span>{formatCount(result.fileCount)} files · {formatScanSize(result)} · {result.warnings.length} warnings</span></div><time>{formatDateTime(result.scannedAt)}</time></section>
          <CategoryTable result={result} limit={24} />
          {result.warnings.length > 0 && <section className="feature-surface warning-list"><h2>Skipped safely</h2>{result.warnings.map((warning) => <p key={warning}><Warning24Regular />{warning}</p>)}</section>}
        </>
      )}
    </div>
  );
}

export function StorageView({ result, scanning, progress, onScan, onChooseFolder }) {
  if (scanning) return <ScanProgress progress={progress} />;
  if (!result) return <EmptyScan onScan={onScan} onChooseFolder={onChooseFolder} />;
  const displayed = result.categories.slice(0, 12);
  const total = Math.max(displayed.reduce((sum, item) => sum + item.sizeBytes, 0), 1);
  return (
    <div className="feature-view">
      <FeatureHeader
        eyebrow="Storage explorer"
        title="What is taking the space"
        description={result.root}
        action={<button className="secondary-button" type="button" onClick={onChooseFolder}><Folder24Regular /> Choose another folder</button>}
      />
      <section className="feature-surface storage-map">
        <div className="surface-heading"><div><h2>Storage map</h2><p>Area is proportional to the folder’s scanned size.</p></div><strong>{formatScanSize(result)}</strong></div>
        <div className="treemap" role="img" aria-label="Proportional storage map">
          {displayed.map((category, index) => (
            <div className={`treemap-node treemap-tone-${index % 5}`} key={category.path} style={{ flexGrow: Math.max(category.sizeBytes / total, 0.025), flexBasis: `${Math.max((category.sizeBytes / total) * 100, 8)}%` }}>
              <strong>{category.name}</strong><span>{formatBytes(category.sizeBytes)}</span><small>{category.lastUsedDays == null ? "Activity unknown" : `Last activity ${category.lastUsedDays} days ago`}</small>
            </div>
          ))}
        </div>
      </section>
      <CategoryTable result={result} limit={24} />
    </div>
  );
}

export function DuplicatesView({ result, scanning, progress, onScan, onChooseFolder }) {
  if (scanning) return <ScanProgress progress={progress} />;
  if (!result) return <EmptyScan onScan={onScan} onChooseFolder={onChooseFolder} />;
  const reclaimable = result.duplicateGroups.reduce((sum, group) => sum + group.reclaimableBytes, 0);
  return (
    <div className="feature-view">
      <FeatureHeader eyebrow="Duplicates" title={`${formatBytes(reclaimable)} in exact copies`} description="Luna compares size first, then BLAKE3 content hashes. No duplicate is selected for deletion automatically." action={<button className="secondary-button" type="button" onClick={onChooseFolder}>Scan another folder</button>} />
      {result.duplicateGroups.length === 0 ? <section className="empty-state-surface"><DocumentCopy24Regular /><h2>No exact duplicates in this scan</h2><p>Try a broader folder or drive if you want to search elsewhere.</p></section> : (
        <section className="feature-surface duplicate-list">
          {result.duplicateGroups.map((group, index) => (
            <details key={group.contentHash} open={index === 0}>
              <summary><span><DocumentCopy24Regular /></span><div><strong>{group.files[0]?.name || "Duplicate group"}</strong><small>{group.files.length} identical files · hash {group.contentHash.slice(0, 12)}…</small></div><b>{formatBytes(group.reclaimableBytes)} reclaimable</b></summary>
              <div>{group.files.map((file) => <div className="duplicate-file" key={file.path}><span>{file.path}</span><small>{file.lastUsedDays == null ? "Activity unknown" : `${file.lastUsedDays} days ago`}</small></div>)}</div>
            </details>
          ))}
        </section>
      )}
    </div>
  );
}

export function LargeFilesView({ result, scanning, progress, onScan, onChooseFolder }) {
  if (scanning) return <ScanProgress progress={progress} />;
  if (!result) return <EmptyScan onScan={onScan} onChooseFolder={onChooseFolder} />;
  return (
    <div className="feature-view">
      <FeatureHeader eyebrow="Large files" title="The files with the biggest footprint" description="Sorted by size. Luna shows these for investigation only and never preselects them for cleanup." action={<button className="secondary-button" type="button" onClick={onChooseFolder}>Choose another folder</button>} />
      <section className="feature-surface large-file-table">
        <div className="data-table"><div className="data-header"><span>File</span><span>Size</span><span>Activity</span></div>{result.largeFiles.map((file) => <div className="data-row" key={file.path}><div className="path-cell"><Folder24Regular /><span><strong>{file.name}</strong><small>{file.path}</small></span></div><strong>{formatBytes(file.sizeBytes)}</strong><span>{file.lastUsedDays == null ? "Unknown" : `${file.lastUsedDays} days ago`}</span></div>)}</div>
      </section>
    </div>
  );
}

export function ScheduleView({ schedule, roots, selectedRoot, onScheduleChange, onCapture }) {
  const enabled = schedule?.enabled || false;
  const frequency = schedule?.frequency || "Weekly";
  const scheduleRoot = schedule?.scanRoot || selectedRoot || roots[0]?.path || "";
  const busy = schedule?.isScanning || false;
  return (
    <div className="feature-view narrow-feature">
      <FeatureHeader eyebrow="Schedule" title="A quiet storage check-in" description="Capture a compact snapshot on your preferred cadence. Luna can scan from the tray without keeping the full interface in memory." action={<button className="secondary-button" type="button" disabled={busy} onClick={onCapture}><ArrowSync24Regular /> {busy ? "Capturing…" : "Capture now"}</button>} />
      <section className="feature-surface settings-list">
        <div className="setting-row"><span className="setting-icon"><CalendarClock24Regular /></span><div><strong>Scheduled snapshots</strong><small>{enabled ? (schedule?.nextRunAt ? `Next capture ${formatDateTime(schedule.nextRunAt)}` : `${frequency} snapshots enabled.`) : "Off until you choose to enable it."}</small></div><button className={`switch ${enabled ? "is-on" : ""}`} type="button" role="switch" aria-checked={enabled} onClick={() => onScheduleChange({ enabled: !enabled, frequency, scanRoot: scheduleRoot })}><i /></button></div>
        <div className="setting-row"><span className="setting-icon"><ArrowSync24Regular /></span><div><strong>Frequency</strong><small>No automatic cleanup—snapshots and reports only.</small></div><select value={frequency} onChange={(event) => onScheduleChange({ enabled, frequency: event.target.value, scanRoot: scheduleRoot })} disabled={!enabled}><option>Daily</option><option>Weekly</option><option>Monthly</option></select></div>
        <div className="setting-row"><span className="setting-icon"><HardDrive24Regular /></span><div><strong>Scan location</strong><small>{scheduleRoot || "Choose a location in Settings"}</small></div><select value={scheduleRoot} onChange={(event) => onScheduleChange({ enabled, frequency, scanRoot: event.target.value })} disabled={!enabled}>{roots.map((entry) => <option key={entry.id} value={entry.path}>{entry.name}</option>)}</select></div>
      </section>
      <div className="privacy-callout"><ShieldCheckmark24Regular /><div><strong>Scheduled cleanup stays off</strong><p>Luna never removes files in the background. Closing the window releases its WebView; the small Rust tray process stays available for your next due snapshot.</p></div></div>
      {schedule?.lastError && <div className="scan-error"><Warning24Regular /><span>{schedule.lastError}</span></div>}
    </div>
  );
}

export function SettingsView({ roots, selectedRoot, onRootChange, onScan, onChooseFolder, startupEnabled, startupBusy, onStartupToggle, aiStatus, onSaveApiKey, onRemoveApiKey, updateState, updateCheckIntervalMinutes, onCheckForUpdates, onUpdateCheckIntervalChange, onInstallUpdate }) {
  const [diagnostics, setDiagnostics] = useState(false);
  const [apiKey, setApiKey] = useState("");
  const [credentialBusy, setCredentialBusy] = useState(false);
  const [credentialMessage, setCredentialMessage] = useState("");
  const root = useMemo(() => roots.find((entry) => entry.path === selectedRoot), [roots, selectedRoot]);
  const credentialSource = aiStatus?.source === "windowsCredentialManager"
    ? "Saved securely in Windows Credential Manager"
    : aiStatus?.source === "environment"
      ? "Using OPENAI_API_KEY from the development environment"
      : "No key configured";
  const updateBusy = ["checking", "downloading", "installing", "restarting"].includes(updateState?.phase);

  async function submitApiKey(event) {
    event.preventDefault();
    if (!apiKey.trim() || credentialBusy) return;
    const submitted = apiKey;
    setApiKey("");
    setCredentialBusy(true);
    setCredentialMessage("Validating with OpenAI…");
    try {
      await onSaveApiKey(submitted);
      setCredentialMessage("Validated and saved. The key is never stored in Luna's settings files.");
    } catch (error) {
      setCredentialMessage(String(error));
    } finally {
      setCredentialBusy(false);
    }
  }

  async function removeSavedApiKey() {
    if (credentialBusy) return;
    setCredentialBusy(true);
    setCredentialMessage("");
    try {
      await onRemoveApiKey();
      setCredentialMessage("The saved Windows credential has been removed.");
    } catch (error) {
      setCredentialMessage(String(error));
    } finally {
      setCredentialBusy(false);
    }
  }

  return (
    <div className="feature-view narrow-feature">
      <FeatureHeader eyebrow="Settings" title="Private by default" description="Control where Luna scans and what metadata leaves your PC." />
      <section className="feature-surface settings-list">
        <div className="setting-row"><span className="setting-icon"><HardDrive24Regular /></span><div><strong>Default scan location</strong><small>{root?.path || selectedRoot || "Home folder"}</small></div><select value={selectedRoot} onChange={(event) => onRootChange(event.target.value)}>{roots.map((entry) => <option key={entry.id} value={entry.path}>{entry.name}</option>)}</select></div>
        <div className="setting-row"><span className="setting-icon"><Folder24Regular /></span><div><strong>Custom folder</strong><small>Choose any accessible folder for a one-time scan.</small></div><button className="secondary-button" type="button" onClick={onChooseFolder}>Choose</button></div>
        <div className="setting-row"><span className="setting-icon"><CalendarClock24Regular /></span><div><strong>Start with Windows</strong><small>Start hidden in the tray; the full window stays unloaded until you open it.</small></div><button className={`switch ${startupEnabled ? "is-on" : ""}`} type="button" role="switch" aria-checked={startupEnabled} disabled={startupBusy} onClick={onStartupToggle}><i /></button></div>
        <div className="setting-row"><span className="setting-icon"><Settings24Regular /></span><div><strong>Share anonymous diagnostics</strong><small>Off by default. No file names or paths.</small></div><button className={`switch ${diagnostics ? "is-on" : ""}`} type="button" role="switch" aria-checked={diagnostics} onClick={() => setDiagnostics(!diagnostics)}><i /></button></div>
      </section>
      <section className="feature-surface update-card">
        <div className="update-heading">
          <span className={`update-state ${updateState?.phase === "available" ? "has-update" : ""}`}><ArrowDownload24Regular /></span>
          <div>
            <h2>Windows updates</h2>
            <p>{updateState?.message || `Version ${updateState?.currentVersion || "0.8.0"} · signed release channel`}</p>
          </div>
          <div className="update-actions">
            <button className="secondary-button" type="button" disabled={updateBusy} onClick={onCheckForUpdates}>{updateState?.phase === "checking" ? "Checking…" : "Check now"}</button>
            {updateState?.phase === "available" && <button className="primary-button" type="button" onClick={onInstallUpdate}>Install {updateState.availableVersion}</button>}
          </div>
        </div>
        {["downloading", "installing", "restarting"].includes(updateState?.phase) && <div className="update-progress" aria-label={`Update progress ${updateState.progress || 0}%`}><i style={{ width: `${updateState.progress || 4}%` }} /></div>}
        <label className="update-interval" htmlFor="update-check-interval">
          <span><strong>Check automatically</strong><small>While the full Luna window is open</small></span>
          <select id="update-check-interval" value={updateCheckIntervalMinutes} disabled={updateBusy} onChange={(event) => onUpdateCheckIntervalChange(Number(event.target.value))}>
            <option value={5}>Every 5 minutes</option>
            <option value={15}>Every 15 minutes</option>
            <option value={30}>Every 30 minutes</option>
            <option value={60}>Every hour</option>
            <option value={360}>Every 6 hours</option>
            <option value={1440}>Every day</option>
          </select>
        </label>
        <small>Every installer and update manifest must match Luna's embedded signing key before installation can begin.</small>
      </section>
      <section className="feature-surface credential-card">
        <div className="credential-heading">
          <span className={`credential-state ${aiStatus?.configured ? "is-ready" : ""}`}><LockClosed24Regular /></span>
          <div><h2>OpenAI connection</h2><p>{credentialSource} · {aiStatus?.model || "gpt-5.6-luna"}</p></div>
        </div>
        <form onSubmit={submitApiKey}>
          <label htmlFor="openai-api-key">OpenAI API key</label>
          <div className="credential-input-row">
            <input
              id="openai-api-key"
              type="password"
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder={aiStatus?.configured ? "Enter a new key to replace the current one" : "sk-…"}
              autoComplete="new-password"
              spellCheck="false"
              disabled={credentialBusy}
            />
            <button className="primary-button" type="submit" disabled={!apiKey.trim() || credentialBusy}>{credentialBusy ? "Checking…" : aiStatus?.configured ? "Replace key" : "Save key"}</button>
          </div>
        </form>
        <div className="credential-foot">
          <small>{credentialMessage || "Luna validates the key in Rust, then stores it with your Windows account—not in the WebView or local JSON."}</small>
          {aiStatus?.source === "windowsCredentialManager" && <button type="button" onClick={removeSavedApiKey} disabled={credentialBusy}>Remove saved key</button>}
        </div>
      </section>
      <section className="feature-surface privacy-details"><div><LockClosed24Regular /><h2>Scan metadata stays local</h2></div><p>Names, paths, sizes, timestamps, and duplicate hashes are processed by the Rust backend. AI reporting is a separate, explicit action and receives a minimized summary rather than file contents.</p><button className="primary-button" type="button" onClick={onScan}>Scan selected location <ArrowRight24Regular /></button></section>
      <div className="model-note"><Sparkle24Regular /><span><strong>AI privacy</strong> Reports send minimized aggregate scan totals only after you explicitly ask Luna to investigate.</span></div>
    </div>
  );
}
