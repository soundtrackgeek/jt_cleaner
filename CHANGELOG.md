# Changelog

## [0.8.0] - 2026-07-14

### Added

- Check the signed release channel automatically while the full window is open, every five minutes by default, with a persisted cadence configurable from five minutes to one day in Settings.
- Show a dismissible update-available toast with an **Update** button that starts the existing signed download and installation flow.

## [0.7.2] - 2026-07-14

### Fixed

- Save and restore the default scan location after closing the window, quitting the tray app, or relaunching Luna Clean.

## [0.7.1] - 2026-07-14

### Fixed

- Live whole-drive scan and trend-capture progress now shows Windows-reported used space and total capacity instead of the logical file-length sum, which could exceed the drive's capacity before the scan completed.

## [0.7.0] - 2026-07-14

### Added

- Remember and restore the main window's position, size, and maximized state when reopening from the tray, restarting the app, or relaunching after an update.
- Ignore a saved position when its window rectangle no longer intersects an available monitor, allowing Windows to place the window safely after a display-layout change.

## [0.6.3] - 2026-07-14

### Fixed

- Trend history now treats Windows display paths such as `C:\` and canonical extended paths such as `\\?\C:\` as the same scan root, while preserving and migrating snapshots saved under the earlier identity.
- Snapshot actions on Trends now show an immediate capturing state, live measured totals, and disabled duplicate actions until the scan finishes.

## [0.6.2] - 2026-07-14

### Fixed

- Whole-drive scans now show Windows-reported used space and total capacity instead of presenting the logical sum of file lengths, which can exceed an NTFS volume's capacity when hard-linked files appear through multiple paths.
- Drive trend snapshots, scheduled-scan notifications, and AI report totals now use the same Windows-reported used-space measurement.

## [0.6.1] - 2026-07-14

### Fixed

- Allow Tauri's dedicated updater restart exit code through the close-to-tray guard so a completed in-app update can relaunch immediately.
- Added a native regression test covering ordinary close-to-tray, explicit quit, and updater restart behavior.

## [0.6.0] - 2026-07-14

### Added

- Signed in-app update checks, download progress, passive Windows installation, and app relaunch from Settings.
- Automatic update checks when the full window opens; the tray-only background process performs no update polling.
- GitHub Actions quality checks for frontend builds, Rust formatting, native tests, and synchronized release versions.
- Automatic signed NSIS builds, updater signatures, `latest.json`, version tags, and GitHub Releases on every push to `master`.
- A release-version guard that keeps `package.json`, Cargo, Tauri configuration, and the dated changelog heading synchronized.

### Changed

- Tauri bundles now emit signed updater artifacts alongside the NSIS installer.
- Windows updates use passive installer mode and require Luna's embedded public key before installation.
- Dynamic imports keep updater code out of the main frontend chunk until an update check is needed.

## [0.5.0] - 2026-07-14

### Added

- GPT-5.6-Luna storage investigations using OpenAI's Responses API and strict structured reports.
- Explicit report and follow-up actions that send only capped aggregate scan metadata, never file contents.
- Masked OpenAI API key setup inside Settings with Rust-side validation before saving.
- Secure per-user key storage in Windows Credential Manager, with the development environment retained as a fallback.
- AI status reporting that identifies whether Luna is using a saved Windows credential or an environment key.
- Native tests for report schema and API key input safeguards, plus an opt-in live API smoke test.

### Changed

- Windows Credential Manager now takes priority over `OPENAI_API_KEY` for normal app use.
- The Trends storage story can be refreshed with an evidence-backed GPT-5.6 report.
- Missing AI configuration now routes directly to Settings instead of leaving the user at a blocked report action.

## [0.4.0] - 2026-07-14

### Added

- Native Windows tray menu for opening Luna Clean, capturing a snapshot, and quitting completely.
- Optional startup with Windows through the Tauri autostart plugin.
- Persisted daily, weekly, or monthly snapshot scheduling with weekly as the default.
- Background snapshot events and retry metadata for the Schedule interface.
- A shared single-scan guard that prevents foreground and scheduled scans from overlapping.
- Rust tests for disabled and immediately due schedule states.

### Changed

- The main WebView is now created on demand and destroyed when its window closes, leaving the smaller Rust tray process running.
- Hidden startup no longer creates the main WebView, reducing measured debug-build tray idle memory to roughly 3 MB private memory on the development machine.
- Scheduled scans capture reports and trends only; cleanup always requires the foreground confirmation flow.

## [0.3.0] - 2026-07-14

### Added

- Compact per-drive storage snapshots derived from completed scan aggregates.
- Storage composition over time using a stacked category chart with exact-value hover details.
- Fastest-mover rankings and an age-cohort heatmap across the retained history.
- Local storage-story insights for total growth, the fastest category, older storage, and duplicate opportunity.
- Rust commands to load and clear history for a selected root.
- Rust tests for same-day replacement and the two-year weekly retention cap.

### Changed

- Repeated scans on the same calendar day now refresh one snapshot instead of creating duplicates.
- Trend history is capped at 104 entries per root and retains only compact totals, not full file inventories.
- Trend charts load on demand so the standard cleanup experience avoids their frontend cost.

## [0.2.0] - 2026-07-14

### Added

- Native folder and drive discovery with Tauri directory selection.
- Rust storage scanner with progress events, top-level category aggregation, large-file ranking, and bounded warning reporting.
- File activity-age buckets using last-access time with modification-time fallback.
- Exact duplicate discovery using size pre-grouping and BLAKE3 content hashes.
- Conservative browser, Codex, and Windows temporary-cache discovery.
- Rust cleanup command restricted to validated cache category roots.
- Functional Overview, Scan results, Storage explorer, Duplicates, Large files, Schedule, and Settings surfaces.
- Rust tests for age classification and cleanup-category rejection.

### Changed

- Connected the selected cleanup-review UI to real scan results when running inside Tauri.
- Review-sensitive duplicates and old Downloads now remain unselected and report-only.
- Expanded the roadmap with compact trend snapshots and low-memory Windows tray operation.

## [0.1.0] - 2026-07-14

### Added

- Initial Rust and Tauri 2 desktop application foundation.
- Selected Luna Clean cleanup-review interface with responsive Fluent styling.
- Interactive cleanup selection, evidence expansion, confirmation, and AI follow-up states.
- Native Windows application configuration and NSIS bundle target.
- Generated Luna Clean application mark and Microsoft Fluent UI icon set.
- Secure local environment template with GPT-5.6-Luna as the planned reporting model.
