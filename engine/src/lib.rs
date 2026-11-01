//! # DEP Engine (dep_engine)
//!
//! DEP SQL on Apache DataFusion: a prototype of the DEP (Database-Enforced
//! Privacy) design proposal by Reuven Mandelovich.
//!
//! A statement goes through four stages:
//!
//! 1. **Parse.** sqlparser-rs, plus the DEP extensions in [`sqltext`]
//!    (`PROTECTED MATCH DOMAIN`, `DISCLOSES`, `LINKS`, `CREATE ENDPOINT`).
//! 2. **Plan.** DataFusion's SQL planner builds a logical plan.
//! 3. **DEP analysis.** [`analyze`] follows protected and disclosed values
//!    through the plan, rejects what the rules forbid, and rewrites the rest:
//!    equality on protected columns becomes equality on keyed index entries,
//!    REVEAL becomes decryption (only inside a DISCLOSES function), and
//!    protected columns in the output become NULL.
//! 4. **Execute.** DataFusion optimizes and runs the rewritten plan.
//!
//! ```no_run
//! # async fn demo() {
//! let server = dep_engine::DepServer::new();
//! let admin = server.connect("operational").unwrap();
//! admin.execute_strict("CREATE TABLE people (id BIGINT, nid TEXT PROTECTED MATCH DOMAIN person_id)").await.unwrap();
//! admin.execute_strict("INSERT INTO people VALUES (1, '123')").await.unwrap();
//! let out = admin.execute_one("SELECT id, nid FROM people WHERE nid = '123'").await.unwrap();
//! # }
//! ```

pub mod analyze;
pub mod audit;
pub mod catalog;
pub mod client;
pub mod crypto;
pub mod error;
pub mod format;
pub mod functions;
pub mod http;
pub mod persist;
pub mod privileges;
pub mod server;
pub mod spill;
pub mod sqltext;
pub mod store;
pub mod tls;
pub mod udf;
pub mod wire;

pub use datafusion;
pub use catalog::PathMode;
pub use error::{DepError, Result};
pub use server::{Connection, DepServer, Output, ServerOptions, Streamed};
