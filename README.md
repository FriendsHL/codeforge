# CodeForge

A coding agent desktop app for macOS — sibling project of skillForge.

## Tech Stack

- **Shell**: [Tauri 2](https://tauri.app) (Rust backend, small bundle, native performance)
- **Frontend**: React 18 + TypeScript + Vite
- **Platform**: macOS (Apple Silicon & Intel)

## Development

Prerequisites: Node.js ≥ 20, Rust toolchain (`rustup`), Xcode Command Line Tools.

```bash
npm install        # install frontend deps
npm run tauri dev  # launch the app in dev mode (hot reload)
```

## Build

```bash
npm run tauri build   # produces a .app / .dmg under src-tauri/target/release/bundle/
```

## Project Layout

```
src/         # React frontend
src-tauri/   # Rust backend (Tauri commands, process management, PTY, etc.)
```
