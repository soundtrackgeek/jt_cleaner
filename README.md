# Luna Clean

Luna Clean is a Rust and Tauri 2 desktop app for understanding and carefully reclaiming storage on Windows 11. Its interface is designed around confidence: safe cache items are separated from files that deserve review, and every cleanup stays behind an explicit confirmation.

## Current release

Version `0.3.0` adds a compact storage-history engine and the selected **Storage composition over time** experience. Every completed scan now captures a tiny aggregate snapshot so Luna can show what is growing, shrinking, and aging without storing a second file inventory.

### Included

- Responsive Windows 11 Fluent-style cleanup review plus Overview, Scan results, Trends, Storage explorer, Duplicates, Large files, Schedule, and Settings surfaces.
- Folder and drive discovery with native directory selection.
- Streaming scan progress from the Rust worker.
- Top-level storage aggregation, large-file ranking, and activity-age buckets.
- Exact duplicate detection using size grouping followed by BLAKE3 content hashes.
- Browser, Codex, and Windows temporary-cache discovery.
- Safe versus review-required cleanup grouping, expandable evidence, and confirmation.
- Native cleanup for known cache roots; old Downloads and duplicates remain review-only.
- Storage composition over time with a stacked category chart, fastest-mover ranking, age-cohort heatmap, and a local narrative summary.
- Per-drive aggregate snapshots containing category totals, age buckets, cleanup signals, and duplicate opportunity—never file contents or a duplicate inventory.
- Same-day snapshot replacement and a 104-snapshot cap per drive, covering roughly two years of weekly history.
- Native Tauri 2 shell and NSIS bundle configuration.

## Prerequisites

- Windows 11 with WebView2.
- Node.js 20 or newer and npm.
- A current Rust MSVC toolchain.
- Visual Studio Build Tools with the Desktop development with C++ workload.

## Setup

```powershell
npm install
Copy-Item .env.example .env
npm run tauri dev
```

Set `OPENAI_API_KEY` in `.env` for the upcoming AI reporting stage. `.env` is ignored by Git and the key is read only by the Rust backend.

## Using the scanner

1. Run `npm run tauri dev`.
2. Open **Scan results**, **Storage explorer**, **Duplicates**, or **Large files**.
3. Choose the default home folder, a detected drive, or **Choose folder** for a custom location.
4. Start the scan and keep the app open while Luna reports progress.
5. Review findings in **Cleanup review**. Safe caches are selected only when data exists; duplicate files and old Downloads are never selected automatically.
6. Open **Trends** after the scan to compare the current snapshot with earlier scans. A second scan on the same day refreshes that day instead of adding noise.

Large drive scans may encounter protected Windows folders. Luna skips unreadable entries, reports bounded warnings, does not follow symbolic links, and excludes common high-churn developer folders such as `.git` and `node_modules`. Duplicate analysis is capped at 20,000 files of at least 1 MB so large scans remain bounded; storage totals are not capped.

## Commands

```powershell
npm run dev          # Browser-based UI development
npm run build        # Build the frontend
npm run check        # Build the frontend and check the Rust crate
npm run tauri dev    # Run the native desktop app
npm run tauri build  # Build the Windows NSIS installer
```

## Safety direction

Luna Clean distinguishes rebuildable caches from personal data, defaults review-sensitive files to unselected, and requires confirmation before removal. The Rust cleanup command accepts category IDs—not arbitrary frontend paths—and revalidates every known cache root before deleting its contents. Trend history stays in Luna's local application-data directory as compact JSON aggregates. The future AI report will receive minimized scan metadata rather than file contents unless a feature explicitly asks for and explains broader access.

## Planned next stages

- Automatic weekly capture through a low-idle-memory system tray with optional Windows startup.
- GPT-5.6-Luna investigation reports and evidence-backed follow-up questions.
