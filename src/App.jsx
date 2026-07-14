import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowSync24Regular,
  Broom24Regular,
  Chat24Regular,
  CheckmarkCircle24Regular,
  ChevronDown20Regular,
  ChevronRight20Regular,
  Clock24Regular,
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

const navigation = [
  { id: "overview", label: "Overview", icon: Home24Regular },
  { id: "scan", label: "Scan results", icon: ScanCamera24Regular },
  { id: "cleanup", label: "Cleanup review", icon: CheckmarkCircle24Regular },
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

function ConfirmDialog({ count, size, onCancel, onConfirm }) {
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
          <button className="secondary-button" type="button" onClick={onCancel}>Keep reviewing</button>
          <button className="primary-button" type="button" onClick={onConfirm}>Clean selected files</button>
        </div>
      </section>
    </div>
  );
}

export function App() {
  const [items, setItems] = useState(initialItems);
  const [expandedId, setExpandedId] = useState("browser-cache");
  const [activeNav, setActiveNav] = useState("cleanup");
  const [confirming, setConfirming] = useState(false);
  const [toast, setToast] = useState("");
  const [followUpOpen, setFollowUpOpen] = useState(false);
  const [question, setQuestion] = useState("");
  const [appVersion, setAppVersion] = useState("0.1.0");

  const selectedItems = useMemo(() => items.filter((item) => item.selected), [items]);
  const selectedSize = useMemo(
    () => selectedItems.reduce((sum, item) => sum + item.size, 0),
    [selectedItems],
  );

  useEffect(() => {
    if (!window.__TAURI_INTERNALS__) return;
    invoke("app_status")
      .then((status) => setAppVersion(status.version))
      .catch(() => undefined);
  }, []);

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

  function completePreviewCleanup() {
    const count = selectedItems.length;
    const size = selectedSize;
    setConfirming(false);
    setItems((current) => current.map((item) => ({ ...item, selected: false })));
    setToast(`Cleanup preview complete — ${formatSize(size)} across ${count} items is ready for the Rust engine.`);
  }

  function submitQuestion(event) {
    event.preventDefault();
    if (!question.trim()) return;
    setToast("Your follow-up is queued for Luna’s GPT-5.6 report connection.");
    setQuestion("");
    setFollowUpOpen(false);
  }

  const safeItems = items.filter((item) => item.group === "safe");
  const reviewItems = items.filter((item) => item.group === "review");

  return (
    <div className="app-shell">
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
              onClick={() => {
                if (item.id === "cleanup") setActiveNav(item.id);
                else setToast(`${item.label} is being connected to the scan engine in the next checkpoint.`);
              }}
            />
          ))}
        </nav>
        <div className="sidebar-spacer" />
        <div className="drive-summary">
          <div>
            <HardDrive24Regular />
            <span>Local Disk (C:)</span>
          </div>
          <small>182 GB free of 476 GB</small>
          <progress value="62" max="100">62%</progress>
        </div>
        <button className="rescan-button" type="button" onClick={() => setToast("A fresh scan will start when the Rust engine is connected.")}>
          <ArrowSync24Regular />
          Rescan
        </button>
        <span className="scan-stamp">Scanned on Jul 14, 2026<br />09:18 AM · v{appVersion}</span>
      </aside>

      <main className="review-workspace">
        <header className="review-header">
          <div>
            <h1>A careful plan to reclaim 18.6 GB</h1>
            <p>We’ve analyzed what you don’t need. Review with confidence—nothing is deleted without confirmation.</p>
            <div className="scan-trust">
              <ShieldCheckmark24Regular />
              <span>Based on scan results only</span>
              <i />
              <time dateTime="2026-07-14T09:18:00">Jul 14, 2026&nbsp;&nbsp; 09:18 AM</time>
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
        <p className="findings-intro">I analyzed your scan results and here’s what stands out.</p>
        <ol className="findings-list">
          {findings.map((finding, index) => (
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
            {[
              ["0.6 GB", "0–30", "days", 14, "mint"],
              ["2.1 GB", "31–90", "days", 28, "green"],
              ["3.4 GB", "91–180", "days", 42, "amber"],
              ["10.3 GB", "180+", "days", 82, "orange"],
            ].map(([value, label, unit, height, color]) => (
              <div className="chart-column" key={label}>
                <span>{value}</span>
                <i className={`bar bar-${color}`} style={{ height: `${height}%` }} />
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
                <button className="primary-button" type="submit">Ask Luna</button>
              </div>
            </form>
          ) : (
            <button className="follow-up-button" type="button" onClick={() => setFollowUpOpen(true)}>
              <Chat24Regular /> Ask a follow-up
            </button>
          )}
          <span>I can explain anything in this plan.</span>
        </div>
      </aside>

      {confirming && (
        <ConfirmDialog
          count={selectedItems.length}
          size={selectedSize}
          onCancel={() => setConfirming(false)}
          onConfirm={completePreviewCleanup}
        />
      )}
      {toast && <div className="toast" role="status">{toast}</div>}
    </div>
  );
}

