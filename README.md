# Luna Clean

Luna Clean is a Rust and Tauri 2 desktop app for understanding and carefully reclaiming storage on Windows 11. Its interface is designed around confidence: safe cache items are separated from files that deserve review, and every cleanup stays behind an explicit confirmation.

## Current release

Version `0.18.0` makes the NTFS inventory loop substantially leaner. Luna now aggregates storage by MFT directory record number, caches exclusion and OneDrive decisions once per directory, borrows file-name bytes directly from the catalogue, and creates a full file path only for the limited duplicate and largest-file candidate sets. Storage explorer totals and exact duplicate verification remain unchanged.

### Included

- Responsive Windows 11 Fluent-style cleanup review plus Overview, Scan results, Trends, Storage explorer, Duplicates, Large files, Schedule, and Settings surfaces.
- Folder and drive discovery with native directory selection.
- A persistent default scan location that is restored whenever the interface opens.
- Automatic restoration of the newest locally saved scan after the window or app restarts, preferring the detailed cache and falling back to aggregate trend history when needed.
- A dated snapshot warning and **Run a new scan** action on restored Scan results, Storage explorer, Duplicates, and Large files views.
- Streaming scan progress from the Rust worker, with Windows-reported drive usage for whole-drive scans, measured bytes for folder scans, and a completed phase-by-phase timing breakdown.
- Automatic MFT-backed inventory for full-drive NTFS scans, including record-number aggregation that avoids constructing a path for every ordinary file, an on-demand Windows UAC relaunch when needed, a safe Windows-directory fallback, and the completed scan method shown in Scan results.
- OneDrive-safe whole-drive inventory using file names, sizes, and Files On-Demand attributes only; online-only placeholders count as 0 local bytes while always-kept and temporarily cached files are reported separately.
- Top-level storage aggregation, selectable large-file ranking, and activity-age buckets.
- Instant Storage explorer drill-down from map tiles and folder rows, including empty folders, direct files, breadcrumbs, and back navigation without a second disk scan.
- Windows-reported used space and total capacity for whole-drive scan summaries and trend snapshots.
- Exact duplicate detection using size grouping, small start/middle/end samples, and then full BLAKE3 hashes only for sample matches, with copy selection that always keeps at least one verified file.
- Confirmed permanent deletion from Duplicates and Large files with scan-bound path, type, size, and duplicate-hash revalidation before removal.
- Browser, Codex, and Windows temporary-cache discovery.
- Safe versus review-required cleanup grouping, expandable per-source locations and measurements, and confirmation.
- Native cleanup for known cache roots; old Downloads remain review-only, while duplicate copies and large files require explicit file-by-file selection.
- Storage composition over time with a stacked category chart, fastest-mover ranking, age-cohort heatmap, and a local narrative summary.
- Per-drive aggregate snapshots containing category totals, age buckets, cleanup signals, and duplicate opportunity—never file contents or a duplicate inventory.
- Immediate Trends capture feedback with measured file progress plus Windows drive usage or scanned folder bytes while the snapshot scan is running.
- Same-day snapshot replacement and a 104-snapshot cap per drive, covering roughly two years of weekly history.
- Snapshot-history inspection with capture totals, file and folder counts, categories, age cohorts, cleanup signals, and duplicate opportunity.
- Confirmed deletion of an individual snapshot without deleting scanned files or clearing other captures.
- Native system tray with **Open Luna Clean**, **Capture storage snapshot**, and **Quit Luna Clean** actions.
- Optional startup with Windows using a hidden `--hidden` launch path.
- Daily, weekly, or monthly background snapshot scheduling with a configurable local capture time; weekly at `09:00` is the default.
- A single-scan guard shared by foreground and scheduled scans.
- Close-to-tray behavior that destroys the WebView instead of keeping the full interface hidden in memory.
- Persistent main-window position, size, and maximized state across tray reopen, app restart, and update relaunch.
- GPT-5.6-Luna investigation reports, follow-up questions, and explicit per-file opinions using minimized metadata.
- Strict structured AI responses with evidence, confidence, risk, and preservation-first next actions.
- Masked in-app OpenAI key setup with Rust-side validation and Windows Credential Manager storage.
- Development fallback through `OPENAI_API_KEY`, with the saved Windows credential taking priority.
- Configurable automatic update checks while the full interface is open, every five minutes by default, plus manual **Check now** in Settings.
- Actionable update-available toasts with a direct **Update** button.
- Signed in-app update download, progress, passive installation, and restart.
- GitHub Actions quality checks and automatic versioned Windows Releases on pushes to `master`.
- Native Tauri 2 shell and NSIS bundle configuration.

## Prerequisites

- Windows 11 with WebView2.
- Node.js 20 or newer and npm.
- A current Rust MSVC toolchain.
- Visual Studio Build Tools with the Desktop development with C++ workload.
- Administrator approval only when Luna prompts for the optional full-drive NTFS catalogue fast path; ordinary folder and fallback scans stay unelevated.

## Setup

```powershell
npm install
Copy-Item .env.example .env
npm run tauri dev
```

For normal use, open **Settings**, enter an OpenAI API key, and choose **Save key**. Luna validates access to the configured model before saving the key in Windows Credential Manager. The masked field is cleared immediately, and the key is never placed in browser storage or Luna's JSON settings.

For development, you can instead set `OPENAI_API_KEY` in `.env`. `.env` is ignored by Git and read only by the Rust backend. A key saved in Windows Credential Manager takes priority over this environment fallback. Set `OPENAI_MODEL` only when testing a compatible alternate model; the default is `gpt-5.6-luna`.

## Using the scanner

1. Run `npm run tauri dev`.
2. Open **Scan results**, **Storage explorer**, **Duplicates**, or **Large files**.
3. Choose the default home folder or a detected drive in **Settings**; Luna remembers that default across restarts. Use **Choose folder** for a one-time custom location.
4. Start the scan and keep the app open while Luna reports progress. When you select an NTFS drive root such as `C:\`, Luna requests administrator approval through Windows UAC only if the catalogue fast path needs it, then restarts and resumes that scan automatically. Scan results identify **NTFS catalogue** when the fast path was used; if it is unavailable, Luna identifies **Windows directories** and continues without failing the scan.
   A whole-drive scan may inventory OneDrive paths so Storage explorer can represent all of `C:\`, but Luna never opens their contents or changes Files On-Demand state. Online-only placeholders contribute 0 local bytes; always-kept and temporarily cached files contribute their current local size.
5. After reopening Luna, those four scan views show the latest saved scan with its date and time. An older aggregate-only snapshot restores Scan Results, top-level Storage explorer totals, and duplicate opportunity; use **Run a new scan** to rebuild drill-down and file-level Duplicates or Large Files lists.
6. In **Storage explorer**, select a folder in either the map or Largest areas list to see the folders and direct files immediately inside it. Use **Back** or an earlier breadcrumb to move up again.
7. Review findings in **Cleanup review**. Expand an item, then select its source count to inspect the scanned locations, measured size, and file count. Safe caches are selected only when data exists; duplicate files, large files, and old Downloads are never selected automatically.
8. In **Duplicates**, select only the exact copies you want to remove. Luna keeps at least one verified copy, re-hashes every selected file before deletion, and lets you ask AI about any copy's location and risk.
9. In **Large files**, select files for permanent deletion or choose **Ask AI** for a conservative verdict and safer storage suggestions based on the selected file's minimized metadata.
10. Open **Trends** after the scan to compare the current snapshot with earlier scans. Capturing from Trends shows progress in place, and a second scan on the same day refreshes that day instead of adding noise. Choose **Review snapshots** to inspect any capture or delete one after confirmation.
11. Choose **Investigate with GPT-5.6-Luna** for an aggregate evidence report, or ask a focused follow-up. AI requests are explicit and do not include file contents.

## Tray and scheduled snapshots

Open **Schedule** to enable a daily, weekly, or monthly aggregate snapshot, choose its local capture time, and select its scan location. The time picker accepts times such as `08:30` and `09:00`; Luna checks for due work every minute while its tray process is running. Existing schedules created before version `0.10.0` use `09:00`. Scheduled scans never clean files. If a scan fails, Luna records the error and waits six hours before retrying rather than looping aggressively.

Open **Settings** to enable **Start with Windows**. Luna then starts hidden in the tray, checks whether a snapshot is due, and keeps the full WebView unloaded until you open the app. Closing the main window returns to that lightweight tray-only state; use **Quit Luna Clean** from the tray to exit completely.

## Updates and releases

Luna checks the GitHub release channel when the full window opens and then every five minutes by default. Open **Settings → Windows updates** to choose a cadence from five minutes to one day or to check manually. The tray-only process remains network-quiet until the full interface opens. If a newer semantic version exists, Luna shows an actionable toast; choosing **Update** downloads the NSIS package, verifies its updater signature against the public key embedded in the app, installs it in passive mode, and relaunches. Installation always requires an explicit button press.

Every push to `master` runs `.github/workflows/release.yml`. The workflow checks synchronized versions, builds the frontend, checks Rust formatting, runs native tests, then creates a signed NSIS installer, updater signature, `latest.json`, `vMAJOR.MINOR.PATCH` tag, and GitHub Release. Before pushing release code, update the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`, then add the matching dated heading to `CHANGELOG.md`; `npm run check:version` enforces this.

The updater private key and its password are stored as `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` GitHub Actions secrets and are never committed. The development machine's backup is outside the repository at `%USERPROFILE%\.tauri\luna-clean.key` with its separate password file; keep a secure offline backup of both because future builds must use the same key to update existing installations.

An elevated scan of a full NTFS drive opens the raw volume read-only, loads `$MFT`, resolves paths through parent record IDs, and processes catalogue metadata in memory. Luna never writes through the raw-volume handle. The UAC relaunch uses Windows' `runas` operation only on Luna's current executable and passes the selected scan root as a quoted argument. Luna exits the unelevated process only after Windows confirms that the elevated copy started. If the volume is not NTFS, the request is for a folder, elevation is declined or unavailable, or the catalogue cannot be parsed safely, Luna keeps the original window open or falls back to Windows directory enumeration as appropriate. The MFT fast path can use significant temporary memory on volumes with unusually large catalogues; that memory is released when inventory collection finishes.

Large drive scans may encounter protected Windows folders. Luna skips unreadable entries, reports bounded warnings, does not follow symbolic links or directory reparse points, and excludes NTFS metadata plus common high-churn developer folders such as `.git` and `node_modules`. OneDrive files are inventory-only: Luna reads catalogue or directory metadata to report locally occupied space, excludes every OneDrive path from duplicate hashing, and refuses file actions against OneDrive results. Duplicate analysis is capped at 20,000 non-cloud files of at least 1 MB so large scans remain bounded; storage totals are not capped.

## Commands

```powershell
npm run dev          # Browser-based UI development
npm run build        # Build the frontend
npm run check:version # Verify synchronized release metadata
npm run check        # Build the frontend and check the Rust crate
npm run tauri dev    # Run the native desktop app
npm run tauri build  # Build the Windows NSIS installer
```

## Safety direction

Luna Clean distinguishes rebuildable caches from personal data, defaults review-sensitive files to unselected, and requires confirmation before removal. Category cleanup accepts only known IDs and revalidates cache roots. File-level commands accept only entries from the latest scan, reject symbolic links and changed files, keep one verified copy per duplicate group, never accept an arbitrary unscanned frontend path, and refuse OneDrive paths. Whole-drive scans use OneDrive metadata only and never request file contents or modify Files On-Demand state. Trend history stays in Luna's local application-data directory as compact JSON aggregates. Separately, Luna keeps one detailed latest-scan cache locally so the scan views can survive a restart; every successful foreground or scheduled scan replaces it. Aggregate AI reports receive capped totals and signals; explicit file reviews receive only the selected file's minimized metadata, with user prefixes redacted or paths made relative to the scan root. File contents are never sent, and OpenAI response storage is disabled for these requests.

## Planned next stages

- Benchmark MFT scan time and peak memory across very large and 4K-native NTFS volumes, then investigate incremental USN-journal refreshes.
- Optional user-defined safe cleanup locations with conservative path validation.
