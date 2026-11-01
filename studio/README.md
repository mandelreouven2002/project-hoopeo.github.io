# Hoopeo Studio

The developer application for DEP SQL, in the spirit of PL/SQL Developer. Tauri 2: the window is a web view
(`ui/`), and everything that touches a database runs in Rust (`src/main.rs`) through DEP Engine.

Every command is in the menu bar:

- **File**: New SQL Window, Open, Save, Save As, Close SQL Window, Exit.
- **Edit**: Undo, Redo, the clipboard commands, Comment / Uncomment (Ctrl+/), Uppercase and Lowercase Selection.
- **View**: Object Browser (Ctrl+B), Output (Ctrl+Shift+O), Refresh Objects (Ctrl+R), and text size.
- **Session**: Connect to DEP Server (host, port, role, endpoint, password, and TLS off, required, or verified
  with a CA file), Open Local Database, New Local Database (in a data directory or in memory, optionally served
  on a port), Serve on Port and Stop Serving, Begin Transaction, Commit (F10), Rollback (Shift+F10),
  Connection Information, Disconnect. The status bar shows when a transaction is open, or failed and needs
  Rollback. To try DEP SQL with sample data, open a new local database and run `examples/clinic.sql` from the
  public repository (File > Open, then Execute).
- **Query**: Execute (F8), Execute Current Statement (Ctrl+Enter), Explain Plan (F5), DEP Notes of Last
  Statement, Clear Output.
- **Tools**: Privacy Matrix, Grants, Roles, Endpoints, Audit Log, Indexes, and the stored form of a table.
- **Window**: Next and Previous SQL Window.
- **Help**: the DEP SQL Reference (F1), the DEP error codes, Licenses and Notices, the project site, About.

The **Output** pane logs every statement with its result, its timing, and its DEP notes; errors show their
SQLSTATE, DEP code, message and hint. Result grids mark protected and disclosed columns. The object browser
shows each table's row count, keys and references.

Build and run (from the root of this repository): `cargo run -p hoopeo-studio` (Linux needs `libwebkit2gtk-4.1-dev`).
Screenshots on Linux: `studio/dev/screenshot.sh output`.

License: Business Source License 1.1 (`../LICENSES/BUSL-1.1.txt`); Help > Licenses and Notices shows it, with the
notice and the third-party notices. Free for individuals, non-profit, educational and research use, and for
non-production use by anyone; a company needs a commercial license for production or commercial use. See `../LICENSING.md`.
