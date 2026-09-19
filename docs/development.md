# Development

Redactio is a pnpm workspace. Install dependencies from the repository root:

```sh
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm build
pnpm tauri:dev
```

The application does not load fonts, scripts, updates, or other resources from the network at runtime. Dependency installation is the only build-time network step.

## Windows 11

Windows 11 x64 is the supported release target. Install Node.js 26, pnpm 10.11, Rust with the stable MSVC toolchain, Microsoft C++ Build Tools with the **Desktop development with C++** workload, and the WebView2 runtime. Run the commands above in PowerShell from a native Windows checkout.

## Linux and WSL development

Linux/WSL is for development checks and is not a release target. Install the Tauri system packages for a WebKitGTK 4.1 build: `build-essential`, `curl`, `wget`, `file`, `libssl-dev`, `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, and `libxdo-dev`. A graphical session such as WSLg is required to interact with `pnpm tauri:dev`; CI-style startup checks can use Xvfb.
