# ANR Native Activation

A small Windows Tauri app for installing, updating, and launching Apex Neural Rendering FG.

## What it does

- Checks the latest ANR FG release on GitHub and downloads its official `full` or `addon` ZIP package.
- Caches release ZIPs under `Documents\ANR Native Act\releases\<tag>`. The app checks the release tag again on the next run and reuses the saved ZIP when it has not changed; the ZIP is only downloaded again if it is missing or its published SHA-256 digest does not match.
- Detects an existing `OptiScaler.ini` and chooses the add-on. Otherwise it installs the full package, which includes OptiScaler.
- Saves backups in `Documents\ANR Native Act\backups` and records installed files so ANR can be removed later.
- Detects game executables and display adapters through Windows. The adapter menu lists integrated and discrete GPUs. Windows exposes power-saving and high-performance routing preferences per game, so selecting a listed adapter applies the matching preference category.
- Reads dedicated VRAM through DXGI instead of WMI's 32-bit `AdapterRAM` field; integrated adapters are labeled as shared memory.
- Launches the selected game executable from its game folder. ANR FG still needs Frame Generation enabled in the game's graphics menu.
- Adds a Monitoring tab with FPS, rolling 1% and 0.1% lows, game GPU busy time, and overall CPU load. It uses display-to-display frame timing when the game reports it, and otherwise falls back to app presents. Each reading can be hidden from the in-game overlay, and the overlay's green/amber/red colors give a quick load and FPS guide.
- Downloads the standalone x64 [PresentMon](https://github.com/GameTechDev/PresentMon) capture tool from its official GitHub releases when monitoring is first used, checks GitHub's SHA-256 digest when provided, then caches it at `Documents\ANR Native Act\tools\PresentMon.exe`. It is not bundled into the installer. If Windows blocks ETW capture, add the Windows account to Performance Log Users or run the app as administrator.
- Displays a compact, always-on-top, click-through overlay over borderless or windowed games. Exclusive fullscreen modes can cover desktop overlays. The 0.1% low is shown after at least 1,000 frames; both low figures use the last 60 seconds of frames.
- Checks for the Microsoft Visual C++ x64 runtime needed by the full package and opens Microsoft's download if it is missing.

The app installs the release package into the game directory; it does not clone ANR's source repository. This matches the upstream release instructions, which distribute ready-to-install ZIP assets.

## Build on Windows

Install Node.js, Rust (stable/MSVC toolchain), and the Microsoft C++ Build Tools with the Windows 10/11 SDK. Then run:

```powershell
npm install
npm run tauri -- dev
```

Create an NSIS installer with:

```powershell
npm run tauri -- build
```

The installer is written under `src-tauri\target\release\bundle\nsis`.

## Notes

The release package README/install scripts are treated as upstream technical references. This app does not execute downloaded `.cmd` or `.ps1` files; it verifies the GitHub-published digest when available, extracts only the expected package directory, checks that the game is closed, backs up replaced files, and applies the required OptiScaler settings itself.

ANR FG is beta software. Supported GPUs are NVIDIA GeForce RTX 20 and newer / GTX 16, AMD Radeon RX 5000 and newer, and Intel Arc A/B, according to the upstream release notes.

**By Ali Soleymani and Codex**
