# Third-party software

Project Hoopeo is built on open-source libraries. Each keeps its own license, and nothing in the licenses of Project Hoopeo (see [`LICENSING.md`](LICENSING.md)) limits the rights those licenses give. This page lists the main libraries of each product, what each is used for, and its license. The complete list of every crate in the binaries, with the full license texts, is in [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES), generated with `cargo about` and shipped with every installer.

The Rust libraries named here are the direct dependencies of each product. DEP Engine, DEP CLI and Hoopeo Studio are distributed as binaries only, and the binaries contain these libraries and their own dependencies, about 590 crate versions across the three products, all under permissive licenses (MIT, Apache 2.0, BSD, ISC, Zlib, CC0, Unicode, CDLA-Permissive 2.0, BSL 1.0), with the exception of a few crates under MPL 2.0, described at the end.

## DEP Engine (and the engine inside DEP CLI and Hoopeo Studio)

| Library | Used for | License |
|---|---|---|
| [Apache DataFusion](https://datafusion.apache.org/) 55, with Apache Arrow | SQL parsing, planning, optimization and execution | Apache-2.0 |
| [redb](https://www.redb.org/) | The embedded key-value store that holds the tables, the indexes and the audit log | MIT OR Apache-2.0 |
| [pgwire](https://github.com/sunng87/pgwire) | The PostgreSQL wire protocol, server side | MIT OR Apache-2.0 |
| [Tokio](https://tokio.rs/) | The asynchronous runtime | MIT |
| [Axum](https://github.com/tokio-rs/axum) | The HTTP API | MIT |
| [rustls](https://github.com/rustls/rustls), [tokio-rustls](https://github.com/rustls/tokio-rustls), [webpki-roots](https://github.com/rustls/webpki-roots) | TLS for the server and the client | Apache-2.0 OR ISC OR MIT; MIT OR Apache-2.0; CDLA-Permissive-2.0 |
| [rcgen](https://github.com/rustls/rcgen) | Self-signed certificates | MIT OR Apache-2.0 |
| [tokio-postgres](https://github.com/sfackler/rust-postgres), tokio-postgres-rustls | The client that connects DEP CLI and Hoopeo Studio to a server | MIT OR Apache-2.0; MIT |
| [hmac](https://github.com/RustCrypto/MACs), [sha2](https://github.com/RustCrypto/hashes) | The blind index (HMAC-SHA256) | MIT OR Apache-2.0 |
| [chacha20poly1305](https://github.com/RustCrypto/AEADs) | Encryption of protected values and of spill files (XChaCha20-Poly1305) | Apache-2.0 OR MIT |
| [rand](https://github.com/rust-random/rand) | Random keys, nonces and generated passwords | MIT OR Apache-2.0 |
| md5, base64, hex, bytes, libc | Password digests, encodings and system calls | MIT or Apache-2.0 |
| serde, serde_json | Serialization of the catalog and of HTTP messages | MIT OR Apache-2.0 |
| chrono, dirs, futures, async-trait | Time, standard directories, asynchronous helpers | MIT OR Apache-2.0 |

## DEP CLI

| Library | Used for | License |
|---|---|---|
| [rustyline](https://github.com/kkawakam/rustyline) | Line editing and history | MIT |
| [rpassword](https://github.com/conradkleinespel/rpassword) | The password prompt | Apache-2.0 |
| DEP Engine, Tokio, dirs | As above | |

## Hoopeo Studio

| Library | Used for | License |
|---|---|---|
| [Tauri](https://tauri.app/) 2, tauri-plugin-dialog, tauri-build | The application shell around the system web view | Apache-2.0 OR MIT |
| [CodeMirror](https://codemirror.net/5/) 5 (with its SQL mode, active-line and match-brackets add-ons) | The SQL editor | MIT |
| DEP Engine, Tokio, serde, serde_json, dirs | As above | |

## DEP.js and the playground

| Library | Used for | License |
|---|---|---|
| [sql.js](https://github.com/sql-js/sql.js) 1.12.0 | SQLite compiled to WebAssembly, which runs the compiled statements | MIT |
| [SQLite](https://sqlite.org/) | Inside sql.js | Public domain |

The license text of sql.js is in [`playground/THIRD_PARTY_LICENSES.md`](playground/THIRD_PARTY_LICENSES.md).

## The website

| Library | Used for | License |
|---|---|---|
| [IBM Plex Sans, IBM Plex Sans Condensed, IBM Plex Mono](https://github.com/IBM/plex) | The fonts of the website and of the playground, served from this repository | SIL Open Font License 1.1 (the license is in `site/assets/fonts/`) |
| [Python-Markdown](https://python-markdown.github.io/) | Builds the pages from Markdown (`site/build.py`); not part of the published site | BSD-3-Clause |
| [Pillow](https://python-pillow.org/) | Makes the web copies of the logos (`site/tools/brand_assets.py`); not part of the published site | HPND |
| [Poppins](https://github.com/itfoundry/poppins) | The typeface in which the wordmark image was drawn; the image is a picture, not a font | SIL Open Font License 1.1 |

The website sends no request to any third party: it has no analytics, no embedded content and no external fonts.

## Crates under the Mozilla Public License 2.0

A few crates in the binaries are under MPL 2.0, a license with file-level copyleft: `option-ext` (through `dirs`, in all three products), and `cssparser`, `cssparser-macros`, `dtoa-short`, `selectors` and `x509-certificate` (through Tauri, in Hoopeo Studio). They are used unmodified, as libraries. The MPL requires that the source of the MPL-covered files be available to recipients; it is, at the repositories named in [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES), and a recipient who wants it may also ask through <https://lp.reuven.click>. Nothing in the MPL reaches the rest of the binary.
