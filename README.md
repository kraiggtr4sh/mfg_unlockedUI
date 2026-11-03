# DLSS Unlocked Manager

Desktop GUI manager (Tauri v2 + Rust) for **[DLSS Unlocked (`ShyVortex/dlss-unlocked`)](https://github.com/ShyVortex/dlss-unlocked)** - providing one-click deployment, upgrades, safe backup/restore, and `OptiScaler.ini` tuning for **Multi-Frame Generation (MFG 2X/3X/4X)**, **DLSS-G**, **XeFG**, and **DLSS-NR (Pre-SR Multipass)**.

## Features

- **Automatic Library Discovery**: Scans **Steam** (`libraryfolders.vdf` + `appmanifest_*.acf`), **Epic Games Launcher**, **GOG**, and custom game folders.
- **Deep Feature & Anti-Cheat Detection**: Automatically finds the game executable folder (`Win64-Shipping.exe`, `bin/x64`, etc.), detects native `nvngx_dlss.dll` / `nvngx_dlssg.dll` / `sl.interposer.dll` / Unreal Engine / RE Engine (`re_chunk_000.pak`), and warns if **EasyAntiCheat** or **BattlEye** is present.
- **GPU Architecture Preset Matching**: Detects NVIDIA Turing/Ampere (`dlssg_sm86` / `AmpereMfgUnlock`), Ada Lovelace (`AdaMfgUnlock`), Blackwell, Intel Arc (`XeFG`), and AMD Radeon (`FSR 3.1 FG`).
- **Safe Manifest & Backup Engine**: 100% compatible with upstream `install.sh` (`.dlss-unlocked.manifest` and `*.dlss-unlocked.bak`), preserving existing mods like ReShade and restoring them cleanly on uninstall.
- **OptiScaler.ini Studio & Editor**: Visual toggles for MFG unlockers, frame generation backend, upscalers, overlay menu, searchable full section editor, and raw `.ini` editor with one-click backup restore.
- **Package Manager**: Download standalone `.zip` releases directly from `ShyVortex/dlss-unlocked` on GitHub or import local `.zip` archives / extracted folders.

## Development & Build

```powershell
# Run in development mode
npm run tauri dev

# Run Rust backend unit tests
cd src-tauri
cargo test

# Build release installer (.exe / .msi)
npm run tauri build
```
