# AddressPincode

A cross-platform desktop app that resolves free-text Indian addresses to PIN codes.

Type a messy address and get the most likely PIN code with ranked alternatives, or feed in an
Excel sheet and have every row resolved in one pass. Matching runs entirely offline against a
bundled dataset of **172,572 post office records** — no network calls, no API keys, nothing leaves
the machine.

Built with [Tauri 2](https://tauri.app) (Rust) and React 19.

---

## Features

**Address lookup** — Enter any free-text address and get the top 5 candidate PIN codes, each with
its post office name, city, district, state and a match score, plus an overall confidence rating.

**Bulk processing** — Point the app at an `.xlsx`/`.xls` file, name the column holding the
addresses, and it resolves every row in parallel with a live progress bar and ETA. The result is
written next to the input as `<filename>_resolved.xlsx`, preserving all original columns and
appending a `Resolved_Pincode` column.

**History** — Every bulk job is recorded in a local SQLite database: filename, row count, duration
and status. Reopen a past output file directly from the app, or delete entries you no longer need.

### How matching works

Addresses are normalised (lowercased, punctuation stripped, common abbreviations such as `rd`,
`ngr`, `dist`, `po` expanded) and then scored against an inverted token index built at startup.
The scorer combines exact phrase matches, address- and office-token overlap, post office name
bonuses and fuzzy string similarity, with results memoised in a 20,000-entry LRU cache. Bulk runs
are parallelised across all cores via [rayon](https://github.com/rayon-rs/rayon).

---

## Install

### macOS

1. Download `AddressPincode_0.1.0_aarch64.dmg` (Apple Silicon) from the release or from whoever
   shared it with you.
2. Open the `.dmg` and drag **AddressPincode** into your Applications folder.

> **The build is not code-signed**, so macOS Gatekeeper will block it on first launch with a
> message like *"AddressPincode is damaged and can't be opened"* or *"cannot be opened because the
> developer cannot be verified"*. This is expected for an unsigned app — it is not a sign that the
> file is corrupt. To run it, either:
>
> - **Right-click** (or Control-click) the app in Applications, choose **Open**, then confirm
>   **Open** in the dialog. You only need to do this once.
> - Or clear the quarantine flag from Terminal:
>   ```sh
>   xattr -dr com.apple.quarantine /Applications/AddressPincode.app
>   ```

### Windows and Linux

No prebuilt binaries are published yet. Build from source on the target machine — see
[Building from source](#building-from-source) below.

### First launch

The app reads the bundled 38 MB dataset into memory on startup and shows an **"Initializing
Database…"** progress screen while it does. This takes a few seconds; the UI is usable as soon as
the progress bar completes.

---

## Building from source

Tauri apps **cannot be cross-compiled** — each installer must be built on the operating system it
targets. A macOS machine produces the `.dmg`, a Windows machine produces the `.exe`, and a Linux
machine produces the `.deb`/`.AppImage`. (To produce all three without owning all three, run the
build on CI runners, e.g. GitHub Actions with `tauri-apps/tauri-action`.)

### Common prerequisites

| Requirement | Notes |
| --- | --- |
| [Rust](https://rustup.rs) | Stable toolchain. The crate uses edition 2024, so **Rust 1.85 or newer** is required. |
| [Node.js](https://nodejs.org) | Version 18 or newer. |

Then clone and install the frontend dependencies:

```sh
git clone https://github.com/Shreetej/addresspincode.git
cd addresspincode
npm install
```

### Platform-specific setup

<details>
<summary><strong>macOS</strong></summary>

Install the Xcode Command Line Tools:

```sh
xcode-select --install
```
</details>

<details>
<summary><strong>Windows</strong></summary>

1. Install the **Microsoft C++ Build Tools**. Download the
   [Visual Studio Build Tools installer](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
   and select the **"Desktop development with C++"** workload.
2. Install Rust using the **MSVC** toolchain (the default on Windows via
   [rustup](https://rustup.rs)) — the GNU toolchain is not supported by Tauri.
3. Install **WebView2**. This is preinstalled on Windows 11 and on fully updated Windows 10; if
   missing, grab the
   [Evergreen Bootstrapper](https://developer.microsoft.com/microsoft-edge/webview2/).
</details>

<details>
<summary><strong>Linux (Debian / Ubuntu)</strong></summary>

```sh
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

For Fedora, Arch and other distributions see the
[Tauri prerequisites guide](https://tauri.app/start/prerequisites/).
</details>

### Build

```sh
npm run tauri build
```

The first build compiles the full Rust dependency tree and takes several minutes; later builds are
much faster. Installers are written to `src-tauri/target/release/bundle/`:

| Platform | Output |
| --- | --- |
| macOS | `dmg/AddressPincode_0.1.0_aarch64.dmg` and `macos/AddressPincode.app` |
| Windows | `nsis/AddressPincode_0.1.0_x64-setup.exe` and `msi/AddressPincode_0.1.0_x64_en-US.msi` |
| Linux | `deb/`, `rpm/` and `appimage/` packages |

On an Intel Mac the macOS artifact is named `_x64.dmg` instead. To ship a single binary that runs
on both Apple Silicon and Intel:

```sh
rustup target add x86_64-apple-darwin aarch64-apple-darwin
npm run tauri build -- --target universal-apple-darwin
```

### Development

```sh
npm run tauri dev
```

This starts the Vite dev server on port 1420 and launches the app with hot reload for the
frontend. Changes to Rust code trigger a rebuild and restart.

---

## Project structure

```
addresspincode/
├── src/                      React frontend
│   ├── App.jsx               Single-component UI: search, bulk, history tabs
│   └── main.jsx              React entry point
├── src-tauri/                Rust backend
│   ├── src/main.rs           Matching engine, Tauri commands, SQLite history
│   ├── tauri.conf.json       App metadata and bundle configuration
│   ├── capabilities/         Tauri permission definitions
│   ├── pincode-chinna.csv    Bundled dataset (172,572 post office records)
│   ├── json/                 Supporting PIN code lookup tables
│   └── icons/                App icons for all platforms
└── vite.config.js            Vite config (fixed port 1420 for Tauri)
```

### Tauri commands

The frontend talks to Rust through five commands, all defined in `src-tauri/src/main.rs`:

| Command | Purpose |
| --- | --- |
| `resolve_pincode` | Resolve a single address, returning ranked candidates |
| `bulk_pincode_process` | Resolve an entire spreadsheet, emitting progress events |
| `get_history` | List past bulk jobs |
| `delete_history_item` | Remove a job from history |
| `open_output_file` | Open a generated file in the system file manager |

### Bundled data

Only `pincode-chinna.csv` and three small lookup tables (`state_pin.json`, `district_pin.json`,
`city_pin.json`) are shipped inside the installer — these are the only files the matching engine
reads. The remaining files in `src-tauri/json/` (`village_meta.json`, `village_pin.json`,
`villagespincodes.csv`, and the `*_alias.json` files) are kept in the repository for reference but
are deliberately excluded from `bundle.resources` in `tauri.conf.json`, since including them would
add roughly 108 MB to every installer for no benefit.

---

## Where your data lives

Bulk job history is stored in a SQLite database in the standard per-user application data
directory:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/com.shree.addresspincode/history.db` |
| Windows | `%APPDATA%\com.shree.addresspincode\history.db` |
| Linux | `~/.local/share/com.shree.addresspincode/history.db` |

Resolved output spreadsheets are saved next to their input file and are never moved or deleted by
the app. Nothing is uploaded anywhere.

---

## Troubleshooting

**"Database Load Failed" on startup** — The bundled dataset could not be read, or the history
database could not be created. Check that the app was installed rather than run from inside the
`.dmg`, and that the application data directory above is writable.

**"Column '…' not found in headers"** — The column name entered on the Bulk tab must match a
header in the first row of the first sheet of the spreadsheet. Matching is case-insensitive but
otherwise exact, so check for stray spaces.

**Bulk processing reports "No data rows found"** — The first sheet contains only a header row, or
is empty. The app reads the first sheet of the workbook only.

---

## License

Not currently licensed for redistribution. Contact the author before sharing outside your
organisation.
