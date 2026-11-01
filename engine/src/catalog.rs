//! The DEP catalog: which columns are protected and in which match domain,
//! the declared functions, and the endpoints. DataFusion keeps the data; this
//! keeps what DataFusion does not know about.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use datafusion::logical_expr::LogicalPlan;

use crate::error::{DepError, Result};

/// The suffix of the hidden column that holds a protected value's ciphertext.
pub const CT_SUFFIX: &str = "__ct";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathMode {
    Operational,
    MaskedRead,
}

impl PathMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PathMode::Operational => "OPERATIONAL",
            PathMode::MaskedRead => "MASKED_READ",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColumnMeta {
    pub name: String,
    pub data_type: DataType,
    pub type_name: String,
    /// The match domain, for a PROTECTED column.
    pub domain: Option<String>,
    pub primary_key: bool,
    pub references: Option<(String, String)>,
    /// NOT NULL (always true for a primary key column).
    pub not_null: bool,
}

/// An index on one or more columns of a table. Every index is kept in the data file
/// (key -> row ids), and is used both to find rows and to enforce constraints.
///
/// The key of a PROTECTED column is its blind index entry, so lookups and uniqueness
/// on protected columns never need a plaintext value.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexMeta {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    /// Backs the primary key.
    #[serde(default)]
    pub primary: bool,
    /// Made by the engine (for a constraint, a reference, or a protected column), rather
    /// than by CREATE INDEX. It goes with the table, and cannot be dropped on its own.
    #[serde(default)]
    pub auto: bool,
    /// False for a unique index whose existing rows had duplicate keys when it was built
    /// (a data directory from before 0.5): it is kept, but uniqueness is not enforced
    /// until REINDEX finds no duplicates.
    #[serde(default = "yes")]
    pub enforced: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct TableMeta {
    pub name: String,
    /// The table's id in the data file. It never changes, and is never reused for another
    /// table while the server runs.
    pub tid: u64,
    pub columns: Vec<ColumnMeta>,
    pub indexes: Vec<IndexMeta>,
    /// A TEMPORARY table: kept in memory only, never written to the data directory.
    pub temporary: bool,
}

impl TableMeta {
    pub fn column(&self, name: &str) -> Option<&ColumnMeta> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// The index whose only column is `column`, if there is one.
    pub fn single_index(&self, column: &str) -> Option<&IndexMeta> {
        self.indexes.iter().find(|i| i.columns.len() == 1 && i.columns[0] == column)
    }

    pub fn index(&self, name: &str) -> Option<&IndexMeta> {
        self.indexes.iter().find(|i| i.name == name)
    }

    pub fn primary_key(&self) -> Vec<String> {
        self.indexes.iter().find(|i| i.primary).map(|i| i.columns.clone()).unwrap_or_default()
    }

    /// The schema DataFusion stores: a protected column becomes its index entry
    /// (under the column's own name) and its ciphertext (`name__ct`), both text.
    pub fn physical_schema(&self) -> SchemaRef {
        let mut fields = Vec::new();
        for c in &self.columns {
            if c.domain.is_some() {
                fields.push(Field::new(&c.name, DataType::Utf8, true));
                fields.push(Field::new(format!("{}{CT_SUFFIX}", c.name), DataType::Utf8, true));
            } else {
                fields.push(Field::new(&c.name, c.data_type.clone(), true));
            }
        }
        Arc::new(Schema::new(fields))
    }
}

#[derive(Debug)]
pub struct FunctionMeta {
    pub name: String,
    pub params: Vec<(String, DataType, String)>,
    pub returns: Vec<(String, DataType, String)>,
    pub discloses: Vec<String>,
    pub links: Vec<String>,
    pub body: String,
    pub comment: Option<String>,
    /// The analyzed and rewritten body, with `$n` placeholders for the parameters.
    pub plan: LogicalPlan,
    /// Output columns that carry disclosed values, with their match domain.
    pub disclosed: Vec<(String, String)>,
    /// Whether the function may run on the masked read path.
    pub masked_ok: bool,
    /// The role that declared the function. The body runs with the owner's privileges.
    pub owner: String,
    /// The table columns the body reads, re-checked against the owner at each call.
    pub reads: Vec<(String, String)>,
    /// Tables the body scans without naming a column (for example COUNT(*)).
    pub scans: Vec<String>,
    /// Functions the body calls.
    pub calls: Vec<String>,
    /// The CREATE FUNCTION statement, kept so the function can be re-created when the server restarts.
    pub source: String,
}

impl FunctionMeta {
    pub fn is_declared(&self) -> bool {
        !self.discloses.is_empty() || !self.links.is_empty()
    }
}

impl FunctionMeta {
    pub fn signature(&self) -> String {
        format!("{}({})", self.name, self.params.iter().map(|p| p.2.clone()).collect::<Vec<_>>().join(", "))
    }
}

/// The superuser created with every server.
pub const ADMIN: &str = "admin";
/// The pseudo-role every role belongs to.
pub const PUBLIC: &str = "public";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Role {
    pub name: String,
    pub login: bool,
    pub superuser: bool,
    /// Postgres-style stored password: hex(md5(password || role name)).
    pub password_md5: Option<String>,
    /// The role's limit on protected matches.
    #[serde(default)]
    pub match_limit: crate::audit::RoleLimit,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TablePriv {
    pub select_all: bool,
    pub select_cols: BTreeSet<String>,
    pub insert: bool,
    pub update_all: bool,
    pub update_cols: BTreeSet<String>,
    pub delete: bool,
}

impl TablePriv {
    pub fn is_empty(&self) -> bool {
        !self.select_all && self.select_cols.is_empty() && !self.insert && !self.update_all && self.update_cols.is_empty() && !self.delete
    }
    pub fn describe(&self) -> String {
        let mut v = Vec::new();
        if self.select_all {
            v.push("SELECT".to_string());
        } else if !self.select_cols.is_empty() {
            v.push(format!("SELECT ({})", self.select_cols.iter().cloned().collect::<Vec<_>>().join(", ")));
        }
        if self.insert {
            v.push("INSERT".into());
        }
        if self.update_all {
            v.push("UPDATE".into());
        } else if !self.update_cols.is_empty() {
            v.push(format!("UPDATE ({})", self.update_cols.iter().cloned().collect::<Vec<_>>().join(", ")));
        }
        if self.delete {
            v.push("DELETE".into());
        }
        v.join(", ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainPriv {
    Disclose,
    Link,
}

#[derive(Debug, Default, Clone)]
pub struct Catalog {
    pub tables: BTreeMap<String, TableMeta>,
    pub functions: BTreeMap<String, Arc<FunctionMeta>>,
    pub endpoints: BTreeMap<String, PathMode>,
    pub roles: BTreeMap<String, Role>,
    /// (role, table) -> privileges
    pub table_privs: BTreeMap<(String, String), TablePriv>,
    /// (role, match domain) -> (DISCLOSE, LINK)
    pub domain_privs: BTreeMap<(String, String), (bool, bool)>,
    /// endpoint -> roles that may connect through it
    pub endpoint_grants: BTreeMap<String, BTreeSet<String>>,
    /// function -> roles that may execute it
    pub fn_execute: BTreeMap<String, BTreeSet<String>>,
    /// Functions in the order they were created, so a restart re-creates callees first.
    pub function_order: Vec<String>,
}

impl Catalog {
    pub fn new() -> Self {
        let mut c = Catalog::default();
        c.endpoints.insert("operational".into(), PathMode::Operational);
        c.endpoints.insert("analytics".into(), PathMode::MaskedRead);
        c.roles.insert(ADMIN.into(), Role { name: ADMIN.into(), login: true, superuser: true, password_md5: None, match_limit: Default::default() });
        c
    }

    pub fn is_superuser(&self, role: &str) -> bool {
        self.roles.get(role).map(|r| r.superuser).unwrap_or(false)
    }

    fn privs<'a>(&'a self, role: &'a str, table: &'a str) -> impl Iterator<Item = &'a TablePriv> + 'a {
        [role, PUBLIC].into_iter().filter_map(move |r| self.table_privs.get(&(r.to_string(), table.to_string())))
    }

    pub fn can_select(&self, role: &str, table: &str, column: &str) -> bool {
        self.is_superuser(role) || self.privs(role, table).any(|p| p.select_all || p.select_cols.contains(column))
    }

    pub fn can_select_any(&self, role: &str, table: &str) -> bool {
        self.is_superuser(role) || self.privs(role, table).any(|p| p.select_all || !p.select_cols.is_empty())
    }

    pub fn can_insert(&self, role: &str, table: &str) -> bool {
        self.is_superuser(role) || self.privs(role, table).any(|p| p.insert)
    }

    pub fn can_update(&self, role: &str, table: &str, column: &str) -> bool {
        self.is_superuser(role) || self.privs(role, table).any(|p| p.update_all || p.update_cols.contains(column))
    }

    pub fn can_delete(&self, role: &str, table: &str) -> bool {
        self.is_superuser(role) || self.privs(role, table).any(|p| p.delete)
    }

    pub fn has_domain_priv(&self, role: &str, domain: &str, kind: DomainPriv) -> bool {
        if self.is_superuser(role) {
            return true;
        }
        [role, PUBLIC].iter().any(|r| match self.domain_privs.get(&(r.to_string(), domain.to_string())) {
            Some((d, l)) => match kind {
                DomainPriv::Disclose => *d,
                DomainPriv::Link => *l,
            },
            None => false,
        })
    }

    pub fn can_connect(&self, role: &str, endpoint: &str) -> bool {
        self.is_superuser(role) || self.endpoint_grants.get(endpoint).map(|s| s.contains(role) || s.contains(PUBLIC)).unwrap_or(false)
    }

    pub fn can_execute(&self, role: &str, f: &FunctionMeta) -> bool {
        self.is_superuser(role) || f.owner == role || self.fn_execute.get(&f.name).map(|s| s.contains(role) || s.contains(PUBLIC)).unwrap_or(false)
    }

    pub fn protected_column(&self, table: &str, column: &str) -> Option<&ColumnMeta> {
        self.tables.get(table)?.column(column).filter(|c| c.domain.is_some())
    }

    /// The references to `table` from other tables (and from itself): (child table, child column, referenced column).
    pub fn references_to(&self, table: &str) -> Vec<(String, String, String)> {
        let mut v = Vec::new();
        for t in self.tables.values() {
            for c in &t.columns {
                if let Some((rt, rc)) = &c.references {
                    if rt == table {
                        v.push((t.name.clone(), c.name.clone(), rc.clone()));
                    }
                }
            }
        }
        v
    }

    pub fn table_by_tid(&self, tid: u64) -> Option<&TableMeta> {
        self.tables.values().find(|t| t.tid == tid)
    }

    /// The table that holds an index of this name.
    pub fn index_owner(&self, index: &str) -> Option<&TableMeta> {
        self.tables.values().find(|t| t.index(index).is_some())
    }
}

/// Maps a SQL type name to an Arrow type.
pub fn parse_type(text: &str) -> Result<DataType> {
    let t = text.trim().to_uppercase();
    let base = t.split('(').next().unwrap_or("").trim().to_string();
    Ok(match base.as_str() {
        "TEXT" | "VARCHAR" | "CHAR" | "CHARACTER VARYING" | "CHARACTER" | "STRING" => DataType::Utf8,
        "BIGINT" | "INT8" => DataType::Int64,
        "INT" | "INTEGER" | "INT4" => DataType::Int32,
        "SMALLINT" | "INT2" => DataType::Int16,
        "BOOLEAN" | "BOOL" => DataType::Boolean,
        "DOUBLE" | "DOUBLE PRECISION" | "FLOAT" | "FLOAT8" | "REAL" | "FLOAT4" | "NUMERIC" | "DECIMAL" => DataType::Float64,
        "DATE" => DataType::Date32,
        "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" | "TIMESTAMPTZ" | "TIMESTAMP WITH TIME ZONE" => DataType::Timestamp(TimeUnit::Nanosecond, None),
        _ => return Err(DepError::sql("42704", format!("type \"{}\" is not supported", text.trim().to_lowercase()))),
    })
}

/// Adds the indexes the engine keeps for a table: the primary key, an index for each
/// PROTECTED column (protected values are matched only by equality, so every match can be a
/// lookup), and one for each column that references another table (to check deletes in the
/// referenced table). An existing index on the same column is not duplicated.
pub fn derive_indexes(t: &mut TableMeta) {
    let pk: Vec<String> = t.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect();
    if !pk.is_empty() && !t.indexes.iter().any(|i| i.primary) {
        t.indexes.insert(0, IndexMeta { name: format!("{}_pkey", t.name), columns: pk, unique: true, primary: true, auto: true, enforced: true });
    }
    let wanted: Vec<String> = t.columns.iter().filter(|c| c.domain.is_some() || c.references.is_some()).map(|c| c.name.clone()).collect();
    for c in wanted {
        ensure_index(t, &c);
    }
}

/// Makes sure a column leads an index of its own (for lookups and reference checks).
pub fn ensure_index(t: &mut TableMeta, column: &str) {
    if t.indexes.iter().any(|i| i.columns.len() == 1 && i.columns[0] == column) {
        return;
    }
    let mut name = format!("{}_{}_idx", t.name, column);
    let mut n = 1;
    while t.indexes.iter().any(|i| i.name == name) {
        n += 1;
        name = format!("{}_{}_idx{n}", t.name, column);
    }
    t.indexes.push(IndexMeta { name, columns: vec![column.to_string()], unique: false, primary: false, auto: true, enforced: true });
}
