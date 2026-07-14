# Luna Clean

Luna Clean is a Rust and Tauri 2 desktop app for understanding and carefully reclaiming storage on Windows 11. Its interface is designed around confidence: safe cache items are separated from files that deserve review, and every cleanup stays behind an explicit confirmation.

## Current release

Version `0.10.0` adds a local capture-time picker to scheduled snapshots. Daily, weekly, and monthly schedules can now run at a chosen time such as `08:30` or `09:00`.

### Included

- Responsive Windows 11 Fluent-style cleanup review plus Overview, Scan results, Trends, Storage explorer, Duplicates, Large files, Schedule, and Settings surfaces.
- Folder and drive discovery with native directory selection.
- A persistent default scan location that is restored whenever the interface opens.
- Streaming scan progress from the Rust worker, with Windows-reported drive usage for whole-drive scans and measured bytes for folder scans.
- Top-level storage aggregation, large-file ranking, and activity-age buckets.
- Windows-reported used space and total capacity for whole-drive scan summaries and trend snapshots.
- Exact duplicate detection using size grouping followed by BLAKE3 content hashes.
- Browser, Codex, and Windows temporary-cache discovery.
- Safe versus review-required cleanup grouping, expandable evidence, and confirmation.
- Native cleanup for known cache roots; old Downloads and duplicates remain review-only.
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
- GPT-5.6-Luna investigation reports and follow-up questions using minimized aggregate scan metadata.
- Strict structured AI responses with evidence, confidence, risk, and review-safe next actions.
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
4. Start the scan and keep the app open while Luna reports progress.
5. Review findings in **Cleanup review**. Safe caches are selected only when data exists; duplicate files and old Downloads are never selected automatically.
6. Open **Trends** after the scan to compare the current snapshot with earlier scans. Capturing from Trends shows progress in place, and a second scan on the same day refreshes that day instead of adding noise. Choose **Review snapshots** to inspect any capture or delete one after confirmation.
7. Choose **Investigate with GPT-5.6-Luna** for an aggregate evidence report, or ask a focused follow-up. AI requests are explicit and do not include file contents.

## Tray and scheduled snapshots

Open **Schedule** to enable a daily, weekly, or monthly aggregate snapshot, choose its local capture time, and select its scan location. The time picker accepts times such as `08:30` and `09:00`; Luna checks for due work every minute while its tray process is running. Existing schedules created before version `0.10.0` use `09:00`. Scheduled scans never clean files. If a scan fails, Luna records the error and waits six hours before retrying rather than looping aggressively.

Open **Settings** to enable **Start with Windows**. Luna then starts hidden in the tray, checks whether a snapshot is due, and keeps the full WebView unloaded until you open the app. Closing the main window returns to that lightweight tray-only state; use **Quit Luna Clean** from the tray to exit completely.

## Updates and releases

Luna checks the GitHub release channel when the full window opens and then every five minutes by default. Open **Settings → Windows updates** to choose a cadence from five minutes to one day or to check manually. The tray-only process remains network-quiet until the full interface opens. If a newer semantic version exists, Luna shows an actionable toast; choosing **Update** downloads the NSIS package, verifies its updater signature against the public key embedded in the app, installs it in passive mode, and relaunches. Installation always requires an explicit button press.

Every push to `master` runs `.github/workflows/release.yml`. The workflow checks synchronized versions, builds the frontend, checks Rust formatting, runs native tests, then creates a signed NSIS installer, updater signature, `latest.json`, `vMAJOR.MINOR.PATCH` tag, and GitHub Release. Before pushing release code, update the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`, then add the matching dated heading to `CHANGELOG.md`; `npm run check:version` enforces this.

The updater private key and its password are stored as `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` GitHub Actions secrets and are never committed. The development machine's backup is outside the repository at `%USERPROFILE%\.tauri\luna-clean.key` with its separate password file; keep a secure offline backup of both because future builds must use the same key to update existing installations.

Large drive scans may encounter protected Windows folders. Luna skips unreadable entries, reports bounded warnings, does not follow symbolic links, and excludes common high-churn developer folders such as `.git` and `node_modules`. Duplicate analysis is capped at 20,000 files of at least 1 MB so large scans remain bounded; storage totals are not capped.

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

Luna Clean distinguishes rebuildable caches from personal data, defaults review-sensitive files to unselected, and requires confirmation before removal. The Rust cleanup command accepts category IDs—not arbitrary frontend paths—and revalidates every known cache root before deleting its contents. Trend history stays in Luna's local application-data directory as compact JSON aggregates. AI reporting receives capped category totals, cleanup signals, age buckets, duplicate opportunity, and trend totals—not file contents or a raw file inventory—and OpenAI response storage is disabled for these requests.

## Planned next stages

- Deeper scanner performance profiling across very large NTFS volumes.
- Optional user-defined safe cleanup locations with conservative path validation.
