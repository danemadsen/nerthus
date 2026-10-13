# Data Folder

The anatomy of the Nerthus data folder: `~/.nerthus` on macOS and Linux, `%LOCALAPPDATA%\Nerthus` on Windows. Options › General has a button that opens it.

| Path | Description |
| --- | --- |
| `user.options` | Interface and workspace preferences.* |
| `preferences.options` | Build engine preferences.* |
| `recent-files.mru` | The recently opened files (File › Recent Files).* |
| `recent-files-all.mru` | Every file opened in Nerthus, which the Start window searches.* |
| `locations.mru` | The folders the file dialogs used last. |
| `usage.stats` | Usage statistics of nodes and connections, which power the Predictive System.* |
| `Benchmark.json` | The compute devices' benchmark scores. |
| `onboard.dat` | Marks the first-run setup as done. |
| `cpuonly.dat` | Present while Nerthus is in [CPU-only mode](cpu-only-mode.md). |
| `verbose.dat` | Present while Verbose Logging is set to be on at start. |
| `mcp.dat` | Present while the MCP server is set to start with Nerthus. |
| `Autosaves/` | Autosaves and recovery copies. |
| `Logs/` | All diagnostic and execution logs. |
| `Macros/` | The installed macros.* |
| `Thumbnails/` | The central thumbnail store, when Options › Paths keeps thumbnails there. |
| `Updates/` | Updates downloaded by Nerthus. Delete them to save space. |

## Autosaves/

| Path | Description |
| --- | --- |
| `canyons_2026-01-28_00-19-30.terrain` | A typical file's autosave copy. |
| `canyons_2026-01-28_00-19-30.terrain.reason` | The reason why this file was autosaved. Plain text file. |
| `DISASTER_RECOVERY_canyons_2026-01-29_22-45-41.terrain` | Disaster recovery copy written when Nerthus crashes. |
| `DISASTER_RECOVERY_canyons_2026-01-29_22-45-41.terrain.reason` | Details, when available, for the disaster recovery copy. |
| `Recovery.terrain` | Precautionary recovery copy of the current file. |
| `Recovery2025-12-27_03-18-34.terrain` | The recovery copy a previous session left, kept aside when Nerthus starts. |
| `Migration_canyons_2026-01-28_00-19-30.terrain` | A copy of a file made in an older format, kept before it is migrated. |

## Logs/

| Path | Description |
| --- | --- |
| `2025-12-31_23-36-20.txt` | Normal log output. |
| `2025-12-31_23-36-46--VERBOSE.txt` | Verbose log output, when Verbose Logging is on. |
| `2025-12-31_05-40-41-BUILD.txt` | The log of a command-line build. |
| `2025-12-31_05-41-02-BUILDHOST.txt` | The log of the command-line build host. |
| `CRASH_2026-01-29_22-45-41.txt` | The session's log up to a crash. |
| `CRASH_Error_2026-01-29_22-45-41.txt` | What went wrong in the crash, and where. |

## Updates/

| Path | Description |
| --- | --- |
| `nerthus-windows.zip` | A copy of an update downloaded by Nerthus. |

---

```txt
~/.nerthus/
├── user.options*
├── preferences.options*
├── recent-files.mru*
├── recent-files-all.mru*
├── locations.mru
├── usage.stats*
├── Benchmark.json
├── onboard.dat
├── Autosaves
│   ├── canyons_2026-01-28_00-19-30.terrain
│   ├── canyons_2026-01-28_00-19-30.terrain.reason
│   ├── DISASTER_RECOVERY_canyons_2026-01-29_22-45-41.terrain
│   ├── DISASTER_RECOVERY_canyons_2026-01-29_22-45-41.terrain.reason
│   ├── Recovery.terrain
│   └── Recovery2025-12-27_03-18-34.terrain
├── Logs
│   ├── 2025-12-31_23-36-20.txt
│   ├── 2025-12-31_23-36-46--VERBOSE.txt
│   ├── CRASH_2026-01-29_22-45-41.txt
│   └── CRASH_Error_2026-01-29_22-45-41.txt
├── Macros*
├── Thumbnails
└── Updates
    └── nerthus-windows.zip
```

`* -- these files or folders may be worth backing up`
