# Luna Clean

Luna Clean is a Rust and Tauri 2 desktop app for understanding and carefully reclaiming storage on Windows 11. Its interface is designed around confidence: safe cache items are separated from files that deserve review, and every cleanup stays behind an explicit confirmation.

## Current release

Version `0.1.0` establishes the native Tauri shell and the selected cleanup-review experience. The scan results are realistic preview data in this checkpoint; filesystem scanning and GPT-5.6-Luna reporting are the next implementation stages.

### Included

- Responsive Windows 11 Fluent-style cleanup review.
- Safe versus review-required grouping.
- Selectable cleanup items, expandable evidence, and confirmation flow.
- Luna findings panel with follow-up interaction.
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

Set `OPENAI_API_KEY` in `.env` when the AI reporting stage is enabled. `.env` is ignored by Git and the key is intended to be read only by the Rust backend.

## Commands

```powershell
npm run dev          # Browser-based UI development
npm run build        # Build the frontend
npm run check        # Build the frontend and check the Rust crate
npm run tauri dev    # Run the native desktop app
npm run tauri build  # Build the Windows NSIS installer
```

## Safety direction

Luna Clean will distinguish rebuildable caches from personal data, default review-sensitive files to unselected, and require confirmation before removal. The AI report receives scan metadata rather than file contents unless a future feature explicitly asks for and explains broader access.

