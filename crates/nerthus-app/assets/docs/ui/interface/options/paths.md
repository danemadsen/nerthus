# Paths

The **Paths** tab defines where Nerthus stores and retrieves user data and project files. Properly configuring these paths ensures smooth operation, faster backups, and prevents accidental data loss or corruption.

## **File Thumbnails**

Specifies where thumbnail images for saved files are stored.

* **Don't Save:** Thumbnails are not generated or stored.
* **Central Data Folder:** Thumbnails are stored in a central location within Nerthus’s data folder. _(Recommended)_
* **Next to File:** Thumbnails are saved alongside the corresponding file.

## **User Files**

Specifies the main directory where Nerthus stores:

* Presets
* Screenshots
* Other Nerthus documents

Default path: `~/Documents/Nerthus` (on Windows, `C:\Users\<User>\Documents\Nerthus`)

## **Project Files**

Defines the default directory where Nerthus project files (`*.terrain`) are stored. Default path: the `Projects` folder in the user files folder.

> [!CAUTION]
> **Do not store project or user folders inside the folder Nerthus is installed in.**
>
> Example: If Nerthus is installed in `D:\Nerthus\`, avoid setting your paths to `D:\Nerthus\Docs`.
>
> **Separation of data and installation files** prevents conflicts during updates or reinstalls as well.

> [!TIP]
> **Projects are lightweight**, but **Builds can be large**. 
>
> Storing builds in a separate drive or location is recommended, especially if you frequently back up project folders.

## **Notes**

* These settings only affect the **default save/load locations**. You can still manually select other paths when saving projects or exporting data.
* Ensure the specified folders have full read/write permissions for Nerthus to operate correctly.
* Changing paths does not automatically move existing files; manual migration is required.
