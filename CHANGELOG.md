# Changelog

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
