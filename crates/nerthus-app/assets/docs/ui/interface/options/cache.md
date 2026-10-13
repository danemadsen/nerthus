# Cache

The **Cache** tab manages how Nerthus stores temporary data and autosaves, allowing you to balance memory usage, disk performance, and long-term cleanup.

## **Logs and Autosaves**

* **Delete after 45 days** _(Toggle)_
  * When enabled, automatically deletes log files and autosaves older than **45 days**.
  * Helps prevent disk clutter over time.

## **Smart Cache**

* **Always Cache to Disk on Close**
  * Writes every built preview to disk when a project closes, and reads it back when the project opens again, so the graph need not build again. A project can also ask for this itself: Graph › Bake › Cache to Disk › When closing file.
* **Use Smart Cache**
  * Offloads **2K or 4K cached terrain data** from RAM to disk if unused for the configured **Idle Threshold**.
  * Data is instantly reloaded to RAM when needed again.
  * **Recommended when the cache folder is on a fast SSD.**
* **Idle Threshold (seconds)**
  * Defines how long unused cached data stays in memory before being offloaded to disk.
  * Default: **600 seconds (10 minutes).**

## **Cache Locations**

* **Cache Output (Path)**
  * Sets the location for storing cached data.
  * SSD storage is recommended for faster read/write speeds.
  * Use the folder icon to select a new directory.
* **Save in Central Cache Folder**
  * Chooses which caches are saved in the Cache Output folder. A cache whose toggle is off is saved next to the project file instead, in a `Cache` subfolder.
  * **Smart Cache and Cache to Disk:** The previews the Smart Cache offloads and those cached to disk on close.
  * **Baked Cache:** The results of baked nodes.

## **Notes**

* Enabling **Smart Cache** on SSD can greatly reduce RAM usage without impacting performance.
* If using a slower HDD, keeping Smart Cache **disabled** may be preferable to avoid latency when reloading cached data.
* Large projects can quickly grow the cache folder; periodically clean it if disk space is limited.
