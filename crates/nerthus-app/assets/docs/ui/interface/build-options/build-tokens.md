# Build Path Tokens

Build paths can include *tokens* (angle brackets) and *variables* (square brackets). When you run a build, Nerthus replaces them with real values to form the final output folder.

**Example:** `<Builds>\[Filename]\[+++]`
Builds into a subfolder named after the current file, and increments a numbered folder on each build.

## Token reference

### `<FileLocation>`

Directory of the active `.terrain` file.
Use this to build relative to your project file (portable graphs).

### `<Builds>`

Your configured Builds root folder (from preferences/settings).
Use this as the standard base for all build outputs.

---

### `[Mutation]`

The current Mutation identifier/name.
Useful when iterating variations and keeping outputs separated.

### `[Region]`

Name of the active Region (if one is defined).
Good for region-based exports (e.g., "NorthRidge", "ValleyA"). If no region exists, this may resolve to empty.

### `[Timestamp]`

Current timestamp in file-time format.
Best for guaranteeing unique folders/files per build.

### `[Filename]`

Active file name without extension.
Example: `AlpinePass` (from `AlpinePass.terrain`).

### `[Username]`

Currently logged-in OS username.
Useful in shared environments to avoid collisions.

### `[Domain]`

User domain (if applicable).
Mostly relevant in managed/corporate Windows environments.

### `[Machine]`

Host machine name.
Useful when building on multiple machines.

### `[Time]`

Current time.
Convenient for quick uniqueness (often combined with `[Date]`).

### `[Date]`

Current date.
Common for organizing daily outputs.

### `[Resolution]`

Build resolution (X dimension).
Example: `2048` (for a 2048×2048 build). Useful when exporting multiple resolutions.

---

## Notes on `[+++]`

`[+++]` is an incrementing counter. Each build creates a new folder by increasing the number (e.g., `001`, `002`, `003`…), preserving every build while keeping the path predictable.
