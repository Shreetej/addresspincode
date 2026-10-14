# History database location

## Background

The bulk-job history is kept in a SQLite database. Originally `init_db()` opened it by the
bare relative path `history.db`, which caused two separate problems.

**During development**, the working directory is `src-tauri/`, so the database file was created
inside the directory Tauri's file watcher monitors. Every time a bulk job wrote to it, the watcher
fired and restarted the whole app mid-run.

**In an installed build**, the working directory is not `src-tauri/` — on macOS it is `/`. Opening
`history.db` there fails outright. Because `init_db()` is called from `load_data()` and its error
propagates via `?`, the failure aborted the entire data load: the app reported
*"Database Load Failed"* on startup and was unusable. This affected every packaged build, while
dev builds appeared to work fine.

## Fix

`init_db()` now takes an `&AppHandle` and resolves the per-user application data directory through
Tauri's path API, creating it if necessary:

```rust
let app_data_dir = app_handle
    .path()
    .app_data_dir()
    .map_err(|e| anyhow::anyhow!("failed to resolve app data directory: {}", e))?;
std::fs::create_dir_all(&app_data_dir)?;

let db_path = app_data_dir.join("history.db");
```

If no database exists at the new location but one is left over at the old CWD-relative path, it is
copied across once so no history is lost. A failed migration logs a warning rather than aborting
startup.

`src-tauri/.taurignore` additionally excludes `*.db`, `*.sqlite`, `*.sqlite3`, `*.log`, `*.tmp`
and `*.temp` from the file watcher, as defence in depth against dev-time restarts.

## Resulting location

The database is `history.db`, stored under the bundle identifier `com.shree.addresspincode`:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/com.shree.addresspincode/history.db` |
| Windows | `%APPDATA%\com.shree.addresspincode\history.db` |
| Linux | `~/.local/share/com.shree.addresspincode/history.db` |

## Verification

Confirmed against the packaged `AddressPincode.app` launched with its working directory set to `/`,
reproducing the installed-app condition that previously failed:

```
Starting background data load...
Loaded 172572 records.
Data load complete!
```

`history.db` was created at the path above, and the identifier was checked against
`CFBundleIdentifier` in the built app's `Info.plist`.
