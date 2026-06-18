# Inventory Scenario Data Source

The inventory/scenario dataset is owned by Flow AI and distributed for this
example under the MIT data license in `LICENSE-DATA.md`.

The user-facing distribution path is a versioned static SQLite artifact. Users
seed local state by downloading the published `.target.sqlite.zst` artifact,
verifying SHA-256, and materializing local SQLite files.

New dataset versions are produced through an internal maintainer process.
Private source coordinates, credentials, and regeneration commands are
intentionally omitted from this public example. A maintainer regeneration run
must produce deterministic ordered snapshots, rebuild the SQLite target
database, validate row counts, write manifest/checksum metadata, and publish the
artifact to public object storage before updating `manifest.example.json`.
