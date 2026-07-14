# Changelog

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
