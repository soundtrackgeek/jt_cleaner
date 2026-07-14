import {
  ArrowRight20Regular,
  ArrowSync24Regular,
  ArrowTrending24Regular,
  Calendar24Regular,
  Chat24Regular,
  MoreHorizontal24Regular,
  Sparkle24Regular,
} from "@fluentui/react-icons";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { formatBytes, formatCount } from "../lib/format.js";

const palette = ["#8577ff", "#55d8b0", "#efb74b", "#62a9e8", "#db7d9f"];
const ageRows = [
  ["recentBytes", "0–30 days"],
  ["inactive30To90Bytes", "31–90 days"],
  ["inactive90To180Bytes", "91–180 days"],
  ["inactive180PlusBytes", "180+ days"],
];

const previewDates = [
  "2026-04-28", "2026-05-05", "2026-05-12", "2026-05-19", "2026-05-26", "2026-06-02",
  "2026-06-09", "2026-06-16", "2026-06-23", "2026-06-30", "2026-07-07", "2026-07-14",
];
const previewValues = {
  media: [64, 66, 68, 70, 74, 77, 81, 84, 89, 91, 94, 99],
  apps: [58, 58, 59, 60, 60, 61, 62, 63, 64, 64, 65, 66],
  games: [48, 48, 52, 52, 52, 55, 55, 59, 59, 62, 62, 62],
  work: [34, 36, 37, 39, 41, 43, 45, 47, 49, 51, 53, 55],
};
const gib = (value) => Math.round(value * 1024 ** 3);

export const previewTrendHistory = {
  rootId: "preview-c",
  rootName: "Local Disk (C:)",
  snapshots: previewDates.map((date, index) => {
    const known = Object.values(previewValues).reduce((sum, values) => sum + values[index], 0);
    const other = 38 + index * 0.8;
    return {
      capturedAt: `${date}T09:18:00+02:00`,
      totalBytes: gib(known + other),
      fileCount: 498_000 + index * 6_250,
      folderCount: 43_000 + index * 480,
      categories: [
        { id: "media", name: "Media", sizeBytes: gib(previewValues.media[index]), fileCount: 62_000, lastUsedDays: 19 },
        { id: "apps", name: "Apps", sizeBytes: gib(previewValues.apps[index]), fileCount: 201_000, lastUsedDays: 2 },
        { id: "games", name: "Games", sizeBytes: gib(previewValues.games[index]), fileCount: 98_000, lastUsedDays: 7 },
        { id: "work", name: "Work", sizeBytes: gib(previewValues.work[index]), fileCount: 76_000, lastUsedDays: 11 },
      ],
      ageBuckets: {
        recentBytes: gib(72 + index * 1.4),
        inactive30To90Bytes: gib(54 + index * 0.6),
        inactive90To180Bytes: gib(45 + index * 1.2),
        inactive180PlusBytes: gib(70 + index * 3.2),
        unknownBytes: gib(1),
      },
      cleanupSignals: [],
      duplicateReclaimableBytes: gib(7 + index * 0.25),
    };
  }),
};

function shortDate(value) {
  return new Intl.DateTimeFormat("en", { month: "short", day: "numeric" }).format(new Date(value));
}

function axisBytes(value) {
  return `${Math.round(value)} GB`;
}

function formatDelta(value) {
  const prefix = value > 0 ? "+" : value < 0 ? "−" : "";
  return `${prefix}${formatBytes(Math.abs(value))}`;
}

function TrendTooltip({ active, payload, label, series }) {
  if (!active || !payload?.length) return null;
  return (
    <div className="trend-tooltip">
      <strong>{shortDate(label)}</strong>
      {series.map((entry) => {
        const point = payload.find((item) => item.dataKey === entry.key);
        return (
          <span key={entry.key}>
            <i style={{ background: entry.color }} />
            {entry.name}
            <b>{point ? `${point.value.toFixed(1)} GB` : "—"}</b>
          </span>
        );
      })}
    </div>
  );
}

function EmptyTrends({ onCapture, scanning, progress }) {
  return (
    <>
      <main className="trends-workspace trend-empty-workspace">
        <section className={`trend-empty ${scanning ? "is-capturing" : ""}`} aria-live="polite" aria-busy={scanning}>
          <span>{scanning ? <ArrowSync24Regular /> : <ArrowTrending24Regular />}</span>
          <p>{scanning ? "CAPTURING SNAPSHOT" : "STORAGE TRENDS"}</p>
          <h1>{scanning ? "Scanning your storage now…" : "Your storage story starts with one scan."}</h1>
          <p>{scanning
            ? `${formatCount(progress?.scannedFiles || 0)} files measured · ${formatBytes(progress?.scannedBytes || 0)}. Keep Luna open while this snapshot finishes.`
            : "Luna stores compact totals and category summaries—never a second copy of your files."}</p>
          <button className="primary-button" type="button" disabled={scanning} onClick={onCapture}>
            {scanning ? "Capturing…" : "Capture the first snapshot"}
          </button>
          {scanning && <div className="trend-scan-progress" aria-hidden="true"><i /></div>}
        </section>
      </main>
      <aside className="trend-story-panel empty-story-panel">
        <div className="trend-story-heading"><Sparkle24Regular /><h2>Luna’s storage story</h2></div>
        <p>After a second snapshot, Luna can explain what moved and where older storage is accumulating.</p>
      </aside>
    </>
  );
}

export function TrendsView({ history, onCapture, onAsk, aiReport, aiBusy, scanning, progress }) {
  const resolvedHistory = history || (!window.__TAURI_INTERNALS__ ? previewTrendHistory : null);
  const snapshots = resolvedHistory?.snapshots || [];
  if (!snapshots.length) return <EmptyTrends onCapture={onCapture} scanning={scanning} progress={progress} />;

  const first = snapshots[0];
  const latest = snapshots.at(-1);
  const categoryTotals = new Map();
  for (const snapshot of snapshots) {
    for (const category of snapshot.categories) {
      categoryTotals.set(category.id, (categoryTotals.get(category.id) || 0) + category.sizeBytes);
    }
  }
  const categoryIds = [...categoryTotals.entries()]
    .sort((left, right) => right[1] - left[1])
    .slice(0, 4)
    .map(([id]) => id);
  const categoryNames = new Map();
  snapshots.forEach((snapshot) => snapshot.categories.forEach((item) => categoryNames.set(item.id, item.name)));
  const series = categoryIds.map((id, index) => ({ id, key: `category${index}`, name: categoryNames.get(id) || "Folder", color: palette[index] }));
  series.push({ id: "other", key: "other", name: "Other", color: palette[4] });

  const chartData = snapshots.map((snapshot) => {
    const record = { date: snapshot.capturedAt };
    let selectedBytes = 0;
    series.slice(0, -1).forEach((entry) => {
      const bytes = snapshot.categories.find((category) => category.id === entry.id)?.sizeBytes || 0;
      record[entry.key] = bytes / 1024 ** 3;
      selectedBytes += bytes;
    });
    record.other = Math.max(snapshot.totalBytes - selectedBytes, 0) / 1024 ** 3;
    return record;
  });

  const firstCategories = new Map(first.categories.map((item) => [item.id, item]));
  const movers = latest.categories
    .map((item) => ({
      ...item,
      delta: item.sizeBytes - (firstCategories.get(item.id)?.sizeBytes || 0),
    }))
    .sort((left, right) => Math.abs(right.delta) - Math.abs(left.delta))
    .slice(0, 4);
  const totalDelta = latest.totalBytes - first.totalBytes;
  const staleDelta = latest.ageBuckets.inactive180PlusBytes - first.ageBuckets.inactive180PlusBytes;
  const fastestMover = movers[0];
  const heatMaximum = Math.max(...snapshots.flatMap((snapshot) => ageRows.map(([key]) => snapshot.ageBuckets[key] || 0)), 1);
  const report = aiReport?.report;
  const storyFindings = report?.findings || [
    { title: `${fastestMover?.name || "Your largest folder"} moved fastest`, detail: fastestMover ? `${formatDelta(fastestMover.delta)} across the measured period. It now uses ${formatBytes(fastestMover.sizeBytes)}.` : "Capture another snapshot to compare categories." },
    { title: `Older storage is ${staleDelta >= 0 ? "accumulating" : "clearing"}`, detail: `The 180+ day cohort changed by ${formatDelta(staleDelta)}. Age is a review signal, not an automatic delete rule.` },
    { title: "Duplicate opportunity", detail: `${formatBytes(latest.duplicateReclaimableBytes)} is represented by additional exact copies in the latest scan.` },
  ];

  return (
    <>
      <main className="trends-workspace">
        <header className="trends-header">
          <div>
            <span>STORAGE TRENDS</span>
            <h1>The story of your storage</h1>
            <p>{snapshots.length} compact {snapshots.length === 1 ? "snapshot" : "snapshots"} for {resolvedHistory.rootName}. See what is growing, shrinking, and quietly getting stale.</p>
          </div>
          <button className="secondary-button trend-capture" type="button" disabled={scanning} onClick={onCapture}>
            {scanning ? <ArrowSync24Regular className="capture-spinner" /> : <Calendar24Regular />} {scanning ? "Capturing…" : "Capture now"}
          </button>
        </header>

        <section className="trend-surface composition-card">
          <div className="trend-card-heading">
            <div>
              <h2>Storage composition over time</h2>
              <p>Each layer shows a top-level category from completed scans.</p>
            </div>
            <strong className={totalDelta >= 0 ? "is-growing" : "is-shrinking"}>{formatDelta(totalDelta)} overall</strong>
          </div>
          <div className="composition-chart" aria-label="Stacked area chart showing storage composition over time">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={chartData} margin={{ top: 14, right: 14, left: -14, bottom: 0 }}>
                <CartesianGrid stroke="rgba(196, 209, 230, 0.10)" vertical={false} />
                <XAxis dataKey="date" tickFormatter={shortDate} minTickGap={34} stroke="#526071" tick={{ fill: "#8490a0", fontSize: 10 }} axisLine={false} tickLine={false} />
                <YAxis tickFormatter={axisBytes} width={60} stroke="#526071" tick={{ fill: "#8490a0", fontSize: 10 }} axisLine={false} tickLine={false} />
                <Tooltip content={<TrendTooltip series={series} />} cursor={{ stroke: "rgba(255,255,255,.26)", strokeDasharray: "4 4" }} />
                {series.map((entry) => (
                  <Area key={entry.key} type="monotone" dataKey={entry.key} stackId="storage" stroke={entry.color} fill={entry.color} fillOpacity={0.72} strokeWidth={1.5} animationDuration={650} />
                ))}
              </AreaChart>
            </ResponsiveContainer>
          </div>
          <div className="trend-legend">
            {series.map((entry) => <span key={entry.key}><i style={{ background: entry.color }} />{entry.name}</span>)}
            <small>Hover the chart for exact values</small>
          </div>
        </section>

        <div className="growth-heading">
          <h2>Growth and aging</h2>
          <span>Since {shortDate(first.capturedAt)}</span>
        </div>
        <div className="trend-detail-grid">
          <section className="trend-surface movers-card">
            <div className="trend-card-heading compact-heading">
              <div><h2>Fastest movers</h2><p>Folders with the biggest absolute change.</p></div>
              <ArrowTrending24Regular />
            </div>
            <ol className="movers-list">
              {movers.map((item, index) => (
                <li key={item.id}>
                  <span>{index + 1}</span>
                  <div><strong>{item.name}</strong><small>{formatBytes(item.sizeBytes)} now</small></div>
                  <b className={item.delta >= 0 ? "is-growing" : "is-shrinking"}>{formatDelta(item.delta)}</b>
                </li>
              ))}
            </ol>
          </section>

          <section className="trend-surface age-heatmap-card">
            <div className="trend-card-heading compact-heading">
              <div><h2>Age-cohort heatmap</h2><p>Brighter cells mean more bytes in that age group.</p></div>
              <Calendar24Regular />
            </div>
            <div className="heatmap">
              {ageRows.map(([key, label]) => (
                <div className="heatmap-row" key={key}>
                  <span>{label}</span>
                  <div>
                    {snapshots.map((snapshot) => {
                      const ratio = (snapshot.ageBuckets[key] || 0) / heatMaximum;
                      return <i key={snapshot.capturedAt} title={`${shortDate(snapshot.capturedAt)} · ${formatBytes(snapshot.ageBuckets[key] || 0)}`} style={{ opacity: Math.max(0.13, ratio) }} />;
                    })}
                  </div>
                </div>
              ))}
              <div className="heatmap-axis"><span>{shortDate(first.capturedAt)}</span><span>{shortDate(latest.capturedAt)}</span></div>
            </div>
          </section>
        </div>
      </main>

      <aside className="trend-story-panel">
        <div className="trend-story-heading">
          <Sparkle24Regular />
          <h2>Luna’s storage story</h2>
          <button className="icon-button" type="button" aria-label="More trend actions"><MoreHorizontal24Regular /></button>
        </div>
        <p className="trend-story-intro">{report?.summary || "Here’s what changed across your snapshots."}</p>
        <div className="story-hero">
          <span>{totalDelta >= 0 ? "Storage grew" : "Storage shrank"}</span>
          <strong>{formatDelta(totalDelta)}</strong>
          <small>from {shortDate(first.capturedAt)} to {shortDate(latest.capturedAt)}</small>
        </div>
        <ol className="story-findings">
          {storyFindings.map((finding, index) => (
            <li key={finding.title}><span>{index + 1}</span><div><h3>{finding.title}</h3><p>{finding.detail}</p></div></li>
          ))}
        </ol>
        <div className="trend-story-actions">
          <button className="follow-up-button" type="button" disabled={aiBusy} onClick={onAsk}><Chat24Regular /> {aiBusy ? "Luna is investigating…" : report ? "Refresh Luna’s trend report" : "Ask Luna about this trend"}</button>
          <button className="story-link" type="button" disabled={aiBusy} onClick={onAsk}>Investigate the changes <ArrowRight20Regular /></button>
        </div>
      </aside>
    </>
  );
}
