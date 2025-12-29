# Fix for Application Restart Issue Due to SQLite Database Changes

## Problem Description
The Tauri application was restarting every time the SQLite database file (`pincode_history.db`) was modified. This was happening because:

1. The database file was being created in the current working directory (src-tauri folder) during development
2. Tauri has a hot reload mechanism that watches the src-tauri directory for file changes
3. When the database file was modified (when users processed Excel files), it triggered the file watcher
4. This caused the entire application to restart

## Solution Implemented

### 1. Moved Database to App Data Directory
- Modified the main function to use Tauri path resolution
- Database is now stored in the application's data directory (not in src-tauri)
- Added proper error handling and directory creation

### 2. Added Database Migration
- The application now checks for an existing database in the old location
- If found, it automatically migrates the data to the new location
- This ensures no user data is lost during the migration

### 3. Updated File Watching Configuration
- Updated `.taurignore` file to ignore database files and temporary files
- This provides an additional layer of protection against unwanted restarts

## Key Changes Made

### Code Changes (`src-tauri/src/main.rs`):

1. **Import Updates**:
   ```rust
   use tauri::{Emitter, Manager}; // Added Manager trait
   ```

2. **Database Path Resolution**:
   ```rust
   let app_data_dir = app.path()
       .app_data_dir()
       .expect("Failed to resolve app data directory");
   
   let db_path = app_data_dir.join("pincode_history.db");
   ```

3. **Database Migration Logic**:
   ```rust
   // Check if there's an existing database in the old location and migrate it
   let old_db_path = std::path::Path::new("pincode_history.db");
   if old_db_path.exists() {
       eprintln!("Found existing database in old location. Migrating to app data directory...");
       if let Err(e) = std::fs::copy(old_db_path, &db_path) {
           eprintln!("Warning: Failed to migrate existing database: {}", e);
       } else {
           eprintln!("Successfully migrated existing database.");
       }
   }
   ```

### Configuration Changes (`src-tauri/.taurignore`):

Added rules to ignore database and temporary files:
```
# Ignore database files to prevent unnecessary app restarts during development
*.db
*.sqlite
*.sqlite3

# Ignore log files
*.log

# Ignore temporary files
*.tmp
*.temp
```

## Benefits of the Fix

1. **No More Restarts**: The application no longer restarts when the database is modified
2. **Better Data Persistence**: Database is stored in a proper application data directory
3. **Cross-Platform Compatibility**: Uses Tauri path resolution for proper OS-specific directories
4. **Data Migration**: Automatically migrates existing data without loss
5. **Development Friendly**: Additional protection through `.taurignore` configuration

## Database Location

After the fix, the database will be stored in:
- **Windows**: `%APPDATA%\addresspincode\pincode_history.db`
- **macOS**: `~/Library/Application Support/addresspincode/pincode_history.db`
- **Linux**: `~/.local/share/addresspincode/pincode_history.db`

## Verification

The fix has been tested and verified:
1. ✅ Code compiles without errors
2. ✅ Application starts successfully
3. ✅ Database migration works (existing data is preserved)
4. ✅ Application runs without restart issues

## Usage

Users can continue using the application normally. No additional action is required:
- Existing data is automatically migrated
- New data will be stored in the proper location
- No functional changes to the user interface or features