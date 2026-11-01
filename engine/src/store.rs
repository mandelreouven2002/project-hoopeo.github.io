//! Table storage: rows and indexes in redb, read by DataFusion as streams.
//!
//! Nothing about a table's rows is kept in memory. Each table has, in the data file:
//!
//! ```text
//! rows/{tid}           row id -> encoded row (a compact binary encoding, below)
//! idx/{tid}/{index}    index key -> row ids (one multimap table per index)
//! dep_seq              tid -> the next row id
//! ```
//!
//! TEMPORARY tables have the same layout in a second database that lives in memory only.
//!
//! **Reading.** A statement reads through a [`ReadView`], set on the DataFusion session state:
//! a redb read transaction (a consistent snapshot, which never blocks and is never blocked),
//! and, in a transaction that has written, the transaction's [`TxWriter`] for the tables it
//! wrote. A scan streams rows in batches from a blocking task, so a table never has to fit
//! in memory. A scan is split into one partition per CPU by row id range. When a filter is
//! an equality (or IN list) on the first column of an index, the scan reads only the rows
//! the index points to. For a PROTECTED column the index key is the blind index entry: the
//! DEP rewrite turns `national_id = '...'` into `national_id = <index entry>`, DataFusion
//! folds that to a constant, and the lookup needs no plaintext value.
//!
//! **Writing.** A transaction that writes holds the server's write lock and one redb write
//! transaction (two, if it writes temporary tables) until COMMIT or ROLLBACK. Each row
//! write keeps every index of the table up to date and enforces unique keys and NOT NULL.
//! While savepoints are open, every row write also records the row's previous state, so
//! ROLLBACK TO SAVEPOINT can put it back.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use datafusion::arrow::array::{
    Array, ArrayRef, BooleanArray, BooleanBuilder, Date32Array, Date32Builder, Float64Array, Float64Builder, Int16Array, Int16Builder, Int32Array, Int32Builder,
    Int64Array, Int64Builder, RecordBatch, StringArray, StringBuilder, TimestampNanosecondArray, TimestampNanosecondBuilder, UInt64Builder,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use datafusion::catalog::{CatalogProvider, CatalogProviderList, SchemaProvider, Session};
use datafusion::common::{DataFusionError, Result as DFResult, ScalarValue};
use datafusion::datasource::TableProvider;
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown, TableType};
use datafusion::physical_plan::stream::RecordBatchReceiverStreamBuilder;
use datafusion::physical_plan::streaming::{PartitionStream, StreamingTableExec};
use datafusion::physical_plan::ExecutionPlan;
use redb::{Database, MultimapTableDefinition, MultimapTableHandle, ReadTransaction, ReadableMultimapTable, ReadableTable, ReadableTableMetadata, TableDefinition, WriteTransaction};

use crate::audit::Metrics;
use crate::catalog::{Catalog, IndexMeta, TableMeta, CT_SUFFIX};
use crate::error::{DepError, Result};

/// The hidden column that carries a row's id, in the plans of UPDATE and DELETE.
pub const ROWID: &str = "__dep_rowid";
/// The most keys an IN list may have and still be answered from an index.
const MAX_LOOKUP_KEYS: usize = 4096;

pub const SEQ: TableDefinition<u64, u64> = TableDefinition::new("dep_seq");

pub fn rows_name(tid: u64) -> String {
    format!("rows/{tid}")
}

pub fn index_name(tid: u64, index: &str) -> String {
    format!("idx/{tid}/{index}")
}

pub fn db_err(e: impl std::fmt::Display) -> DepError {
    DepError::sql("58030", format!("data file: {e}"))
}

fn df(e: DepError) -> DataFusionError {
    DataFusionError::External(Box::new(e))
}

// ------------------------------------------------------------------ the row encoding

/// Rows are stored as: a version byte (1), then per column a tag (0 NULL, 1 value) and the
/// value: TEXT as a u32 length and UTF-8 bytes; integers, dates and timestamps little-endian
/// in their width; DOUBLE as 8 bytes; BOOLEAN as one byte. A protected column is stored as
/// two TEXT columns: its index entry and its ciphertext.
const ROW_VERSION: u8 = 1;

fn width(t: &DataType) -> Option<usize> {
    Some(match t {
        DataType::Int64 | DataType::Float64 | DataType::Timestamp(_, _) => 8,
        DataType::Int32 | DataType::Date32 => 4,
        DataType::Int16 => 2,
        DataType::Boolean => 1,
        DataType::Utf8 => return None,
        _ => return None,
    })
}

/// The value slices of an encoded row, one per column (None for NULL).
pub fn split_row<'a>(types: &[DataType], row: &'a [u8]) -> Result<Vec<Option<&'a [u8]>>> {
    let bad = || DepError::sql("XX001", "a stored row could not be decoded");
    if row.first() != Some(&ROW_VERSION) {
        return Err(bad());
    }
    let mut at = 1usize;
    let mut out = Vec::with_capacity(types.len());
    for t in types {
        let tag = *row.get(at).ok_or_else(bad)?;
        at += 1;
        if tag == 0 {
            out.push(None);
            continue;
        }
        let n = match width(t) {
            Some(n) => n,
            None => {
                let len = u32::from_le_bytes(row.get(at..at + 4).ok_or_else(bad)?.try_into().unwrap()) as usize;
                at += 4;
                len
            }
        };
        out.push(Some(row.get(at..at + n).ok_or_else(bad)?));
        at += n;
    }
    Ok(out)
}

/// Encodes every row of a batch whose columns are a table's physical columns.
pub fn encode_rows(batch: &RecordBatch) -> Result<Vec<Vec<u8>>> {
    let n = batch.num_rows();
    let mut out: Vec<Vec<u8>> = (0..n).map(|_| vec![ROW_VERSION]).collect();
    for col in batch.columns() {
        macro_rules! fixed {
            ($t:ty, $conv:expr) => {{
                let a = col.as_any().downcast_ref::<$t>().ok_or_else(|| DepError::sql("XX000", "internal: column type"))?;
                for (r, buf) in out.iter_mut().enumerate() {
                    if a.is_null(r) {
                        buf.push(0);
                    } else {
                        buf.push(1);
                        buf.extend_from_slice(&$conv(a.value(r)));
                    }
                }
            }};
        }
        match col.data_type() {
            DataType::Utf8 => {
                let a = col.as_any().downcast_ref::<StringArray>().ok_or_else(|| DepError::sql("XX000", "internal: column type"))?;
                for (r, buf) in out.iter_mut().enumerate() {
                    if a.is_null(r) {
                        buf.push(0);
                    } else {
                        let v = a.value(r).as_bytes();
                        buf.push(1);
                        buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                        buf.extend_from_slice(v);
                    }
                }
            }
            DataType::Int64 => fixed!(Int64Array, |v: i64| v.to_le_bytes()),
            DataType::Int32 => fixed!(Int32Array, |v: i32| v.to_le_bytes()),
            DataType::Int16 => fixed!(Int16Array, |v: i16| v.to_le_bytes()),
            DataType::Float64 => fixed!(Float64Array, |v: f64| v.to_le_bytes()),
            DataType::Date32 => fixed!(Date32Array, |v: i32| v.to_le_bytes()),
            DataType::Timestamp(TimeUnit::Nanosecond, _) => fixed!(TimestampNanosecondArray, |v: i64| v.to_le_bytes()),
            DataType::Boolean => fixed!(BooleanArray, |v: bool| [v as u8]),
            t => return Err(DepError::sql("XX000", format!("internal: cannot store type {t}"))),
        }
    }
    Ok(out)
}

enum Builder {
    Utf8(StringBuilder),
    I64(Int64Builder),
    I32(Int32Builder),
    I16(Int16Builder),
    F64(Float64Builder),
    Date(Date32Builder),
    Ts(TimestampNanosecondBuilder),
    Bool(BooleanBuilder),
    RowId(UInt64Builder),
}

impl Builder {
    fn new(t: &DataType, cap: usize) -> Result<Self> {
        Ok(match t {
            DataType::Utf8 => Builder::Utf8(StringBuilder::with_capacity(cap, cap * 16)),
            DataType::Int64 => Builder::I64(Int64Builder::with_capacity(cap)),
            DataType::Int32 => Builder::I32(Int32Builder::with_capacity(cap)),
            DataType::Int16 => Builder::I16(Int16Builder::with_capacity(cap)),
            DataType::Float64 => Builder::F64(Float64Builder::with_capacity(cap)),
            DataType::Date32 => Builder::Date(Date32Builder::with_capacity(cap)),
            DataType::Timestamp(TimeUnit::Nanosecond, _) => Builder::Ts(TimestampNanosecondBuilder::with_capacity(cap)),
            DataType::Boolean => Builder::Bool(BooleanBuilder::with_capacity(cap)),
            DataType::UInt64 => Builder::RowId(UInt64Builder::with_capacity(cap)),
            t => return Err(DepError::sql("XX000", format!("internal: cannot read type {t}"))),
        })
    }

    fn push(&mut self, v: Option<&[u8]>) {
        macro_rules! le {
            ($b:expr, $t:ty, $v:expr) => {
                match $v {
                    Some(s) => $b.append_value(<$t>::from_le_bytes(s.try_into().unwrap())),
                    None => $b.append_null(),
                }
            };
        }
        match self {
            Builder::Utf8(b) => match v {
                Some(s) => b.append_value(std::str::from_utf8(s).unwrap_or("")),
                None => b.append_null(),
            },
            Builder::I64(b) => le!(b, i64, v),
            Builder::I32(b) => le!(b, i32, v),
            Builder::I16(b) => le!(b, i16, v),
            Builder::F64(b) => le!(b, f64, v),
            Builder::Date(b) => le!(b, i32, v),
            Builder::Ts(b) => le!(b, i64, v),
            Builder::Bool(b) => match v {
                Some(s) => b.append_value(s[0] != 0),
                None => b.append_null(),
            },
            Builder::RowId(b) => b.append_null(),
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Builder::Utf8(b) => Arc::new(b.finish()),
            Builder::I64(b) => Arc::new(b.finish()),
            Builder::I32(b) => Arc::new(b.finish()),
            Builder::I16(b) => Arc::new(b.finish()),
            Builder::F64(b) => Arc::new(b.finish()),
            Builder::Date(b) => Arc::new(b.finish()),
            Builder::Ts(b) => Arc::new(b.finish()),
            Builder::Bool(b) => Arc::new(b.finish()),
            Builder::RowId(b) => Arc::new(b.finish()),
        }
    }
}

/// Builds record batches of some of a table's columns (and, if asked, the row id) from encoded rows.
pub struct BatchBuilder {
    schema: SchemaRef,
    types: Arc<Vec<DataType>>,
    /// For each physical column, the output column it goes to.
    target: Vec<Option<usize>>,
    rowid_at: Option<usize>,
    builders: Vec<Builder>,
    rows: usize,
    cap: usize,
}

impl BatchBuilder {
    /// `out` lists, in output order, the physical column positions to read; `usize::MAX` stands for the row id.
    pub fn new(info: &TableInfo, out: &[usize], schema: SchemaRef, cap: usize) -> Result<Self> {
        let mut target = vec![None; info.types.len()];
        let mut rowid_at = None;
        for (o, p) in out.iter().enumerate() {
            if *p == usize::MAX {
                rowid_at = Some(o);
            } else {
                target[*p] = Some(o);
            }
        }
        let builders = schema.fields().iter().map(|f| Builder::new(f.data_type(), cap)).collect::<Result<Vec<_>>>()?;
        Ok(BatchBuilder { schema, types: info.types.clone(), target, rowid_at, builders, rows: 0, cap })
    }

    pub fn push(&mut self, rowid: u64, row: &[u8]) -> Result<()> {
        let vals = split_row(&self.types, row)?;
        for (p, v) in vals.into_iter().enumerate() {
            if let Some(o) = self.target[p] {
                self.builders[o].push(v);
            }
        }
        if let Some(o) = self.rowid_at {
            if let Builder::RowId(b) = &mut self.builders[o] {
                b.append_value(rowid);
            }
        }
        self.rows += 1;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    pub fn full(&self) -> bool {
        self.rows >= self.cap
    }

    pub fn finish(&mut self) -> Result<RecordBatch> {
        let cols: Vec<ArrayRef> = self.builders.iter_mut().map(|b| b.finish()).collect();
        let rows = self.rows;
        self.rows = 0;
        let opts = datafusion::arrow::array::RecordBatchOptions::new().with_row_count(Some(rows));
        Ok(RecordBatch::try_new_with_options(self.schema.clone(), cols, &opts)?)
    }
}

// ------------------------------------------------------------------ index keys

/// Index keys sort like their values: a type tag, then integers (and dates and timestamps)
/// as big-endian with the sign bit flipped, doubles in their sortable bit form, and text
/// with 0x00 escaped and a 0x00 0x00 terminator, so keys of several columns concatenate.
fn key_part(t: &DataType, v: &[u8], out: &mut Vec<u8>) {
    match t {
        DataType::Int64 | DataType::Timestamp(_, _) => int_key(i64::from_le_bytes(v.try_into().unwrap()), out),
        DataType::Int32 | DataType::Date32 => int_key(i32::from_le_bytes(v.try_into().unwrap()) as i64, out),
        DataType::Int16 => int_key(i16::from_le_bytes(v.try_into().unwrap()) as i64, out),
        DataType::Float64 => float_key(f64::from_le_bytes(v.try_into().unwrap()), out),
        DataType::Boolean => {
            out.push(0x30);
            out.push(v[0]);
        }
        _ => text_key(v, out),
    }
}

fn int_key(i: i64, out: &mut Vec<u8>) {
    out.push(0x10);
    out.extend_from_slice(&((i as u64) ^ (1u64 << 63)).to_be_bytes());
}

fn float_key(f: f64, out: &mut Vec<u8>) {
    let f = if f == 0.0 { 0.0 } else { f };
    let b = f.to_bits();
    let b = if b >> 63 == 1 { !b } else { b ^ (1u64 << 63) };
    out.push(0x20);
    out.extend_from_slice(&b.to_be_bytes());
}

fn text_key(v: &[u8], out: &mut Vec<u8>) {
    out.push(0x40);
    for b in v {
        out.push(*b);
        if *b == 0 {
            out.push(0xFF);
        }
    }
    out.push(0);
    out.push(0);
}

/// The index key of a literal, for a column of type `t`; None when the literal cannot equal any value of the column.
pub fn scalar_key(t: &DataType, v: &ScalarValue) -> Option<Option<Vec<u8>>> {
    if v.is_null() {
        return Some(None);
    }
    let mut out = Vec::new();
    let int = |v: &ScalarValue| -> Option<i64> {
        Some(match v {
            ScalarValue::Int8(Some(x)) => *x as i64,
            ScalarValue::Int16(Some(x)) => *x as i64,
            ScalarValue::Int32(Some(x)) => *x as i64,
            ScalarValue::Int64(Some(x)) => *x,
            ScalarValue::UInt8(Some(x)) => *x as i64,
            ScalarValue::UInt16(Some(x)) => *x as i64,
            ScalarValue::UInt32(Some(x)) => *x as i64,
            ScalarValue::UInt64(Some(x)) => i64::try_from(*x).ok()?,
            ScalarValue::Float64(Some(f)) if f.fract() == 0.0 && f.abs() < 9.0e18 => *f as i64,
            _ => return None,
        })
    };
    match t {
        DataType::Int64 | DataType::Int32 | DataType::Int16 => int_key(int(v)?, &mut out),
        DataType::Date32 => match v {
            ScalarValue::Date32(Some(d)) => int_key(*d as i64, &mut out),
            _ => return None,
        },
        DataType::Timestamp(_, _) => {
            let ns = match v {
                ScalarValue::TimestampNanosecond(Some(x), _) => *x,
                ScalarValue::TimestampMicrosecond(Some(x), _) => x.checked_mul(1_000)?,
                ScalarValue::TimestampMillisecond(Some(x), _) => x.checked_mul(1_000_000)?,
                ScalarValue::TimestampSecond(Some(x), _) => x.checked_mul(1_000_000_000)?,
                _ => return None,
            };
            int_key(ns, &mut out)
        }
        DataType::Float64 => match v {
            ScalarValue::Float64(Some(f)) => float_key(*f, &mut out),
            ScalarValue::Float32(Some(f)) => float_key(*f as f64, &mut out),
            other => float_key(int(other)? as f64, &mut out),
        },
        DataType::Boolean => match v {
            ScalarValue::Boolean(Some(b)) => {
                out.push(0x30);
                out.push(*b as u8);
            }
            _ => return None,
        },
        DataType::Utf8 => match v {
            ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) | ScalarValue::Utf8View(Some(s)) => text_key(s.as_bytes(), &mut out),
            _ => return None,
        },
        _ => return None,
    }
    Some(Some(out))
}

// ------------------------------------------------------------------ tables as stored

/// An index as the storage layer uses it: its columns as physical positions.
#[derive(Debug, Clone)]
pub struct IndexInfo {
    pub meta: IndexMeta,
    pub cols: Vec<usize>,
}

/// A table as the storage layer uses it.
#[derive(Debug)]
pub struct TableInfo {
    pub meta: TableMeta,
    pub schema: SchemaRef,
    pub types: Arc<Vec<DataType>>,
    pub indexes: Vec<IndexInfo>,
    /// NOT NULL columns: (physical position, column name).
    pub not_null: Vec<(usize, String)>,
}

impl TableInfo {
    pub fn new(meta: &TableMeta) -> Arc<TableInfo> {
        let schema = meta.physical_schema();
        let types: Vec<DataType> = schema.fields().iter().map(|f| f.data_type().clone()).collect();
        let pos = |c: &str| schema.index_of(c).ok();
        let indexes = meta
            .indexes
            .iter()
            .filter_map(|i| Some(IndexInfo { meta: i.clone(), cols: i.columns.iter().map(|c| pos(c)).collect::<Option<Vec<_>>>()? }))
            .collect();
        let not_null = meta.columns.iter().filter(|c| c.not_null).filter_map(|c| Some((pos(&c.name)?, c.name.clone()))).collect();
        Arc::new(TableInfo { meta: meta.clone(), schema, types: Arc::new(types), indexes, not_null })
    }

    pub fn tid(&self) -> u64 {
        self.meta.tid
    }

    pub fn temp(&self) -> bool {
        self.meta.temporary
    }

    /// The key of an index for an encoded row; None when a key column is NULL (NULL keys are not indexed).
    pub fn key(&self, ix: &IndexInfo, row: &[u8]) -> Result<Option<Vec<u8>>> {
        let vals = split_row(&self.types, row)?;
        let mut out = Vec::new();
        for c in &ix.cols {
            match vals[*c] {
                Some(v) => key_part(&self.types[*c], v, &mut out),
                None => return Ok(None),
            }
        }
        Ok(Some(out))
    }

    /// How a key is shown in an error: values of ordinary columns, never of protected ones.
    pub fn describe_key(&self, ix: &IndexInfo, row: &[u8]) -> String {
        let cols = ix.meta.columns.join(", ");
        let protected = ix.meta.columns.iter().any(|c| self.meta.column(c).map(|m| m.domain.is_some()).unwrap_or(false));
        if protected {
            return format!("Key ({cols})");
        }
        let vals = split_row(&self.types, row).unwrap_or_default();
        let shown: Vec<String> = ix.cols.iter().map(|c| show_value(&self.types[*c], vals.get(*c).copied().flatten())).collect();
        format!("Key ({cols})=({})", shown.join(", "))
    }

    /// The value of a column of an encoded row, for messages (ordinary columns only).
    pub fn show(&self, col: usize, row: &[u8]) -> String {
        let vals = split_row(&self.types, row).unwrap_or_default();
        show_value(&self.types[col], vals.get(col).copied().flatten())
    }
}

fn show_value(t: &DataType, v: Option<&[u8]>) -> String {
    let Some(v) = v else { return "NULL".into() };
    match t {
        DataType::Int64 => i64::from_le_bytes(v.try_into().unwrap()).to_string(),
        DataType::Int32 => i32::from_le_bytes(v.try_into().unwrap()).to_string(),
        DataType::Int16 => i16::from_le_bytes(v.try_into().unwrap()).to_string(),
        DataType::Float64 => f64::from_le_bytes(v.try_into().unwrap()).to_string(),
        DataType::Boolean => (v[0] != 0).to_string(),
        DataType::Date32 => {
            let d = i32::from_le_bytes(v.try_into().unwrap());
            (chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap() + chrono::Duration::days(d as i64)).to_string()
        }
        DataType::Timestamp(_, _) => chrono::DateTime::from_timestamp_nanos(i64::from_le_bytes(v.try_into().unwrap())).naive_utc().to_string(),
        _ => String::from_utf8_lossy(v).to_string(),
    }
}

// ------------------------------------------------------------------ reading

/// Where a scan reads a table: a snapshot, or the transaction's own write transaction.
#[derive(Clone)]
pub enum Source {
    Snap(Arc<ReadTransaction>),
    Writer(Arc<Mutex<TxWriter>>),
}

macro_rules! each_row {
    ($table:expr, $from:expr, $to:expr, $max:expr, $f:expr) => {{
        let mut n = 0usize;
        let mut next = None;
        for item in $table.range($from..=$to).map_err(db_err)? {
            let (k, v) = item.map_err(db_err)?;
            let k = k.value();
            if n >= $max {
                next = Some(k);
                break;
            }
            $f(k, v.value())?;
            n += 1;
        }
        Ok::<Option<u64>, DepError>(next)
    }};
}

impl Source {
    /// Calls `f` for up to `max` rows with row ids in `from..=to`, in row id order. Returns the
    /// row id to continue from, if rows are left.
    pub fn rows(&self, info: &TableInfo, from: u64, to: u64, max: usize, f: &mut dyn FnMut(u64, &[u8]) -> Result<()>) -> Result<Option<u64>> {
        let name = rows_name(info.tid());
        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
        match self {
            Source::Snap(r) => match r.open_table(def) {
                Ok(t) => each_row!(t, from, to, max, f),
                Err(redb::TableError::TableDoesNotExist(_)) => Ok(None),
                Err(e) => Err(db_err(e)),
            },
            Source::Writer(w) => {
                let mut w = w.lock().unwrap();
                let wt = w.wt(info.temp())?;
                let t = wt.open_table(def).map_err(db_err)?;
                each_row!(t, from, to, max, f)
            }
        }
    }

    /// The rows with these row ids (missing ones are skipped).
    pub fn get_many(&self, info: &TableInfo, rowids: &[u64], f: &mut dyn FnMut(u64, &[u8]) -> Result<()>) -> Result<()> {
        let name = rows_name(info.tid());
        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
        macro_rules! go {
            ($t:expr) => {{
                for r in rowids {
                    if let Some(v) = $t.get(*r).map_err(db_err)? {
                        f(*r, v.value())?;
                    }
                }
                Ok(())
            }};
        }
        match self {
            Source::Snap(rt) => match rt.open_table(def) {
                Ok(t) => go!(t),
                Err(redb::TableError::TableDoesNotExist(_)) => Ok(()),
                Err(e) => Err(db_err(e)),
            },
            Source::Writer(w) => {
                let mut w = w.lock().unwrap();
                let wt = w.wt(info.temp())?;
                let t = wt.open_table(def).map_err(db_err)?;
                go!(t)
            }
        }
    }

    /// The row ids an index holds for these keys, sorted and without duplicates.
    pub fn lookup(&self, info: &TableInfo, index: &str, keys: &[Vec<u8>]) -> Result<Vec<u64>> {
        let name = index_name(info.tid(), index);
        let def: MultimapTableDefinition<&[u8], u64> = MultimapTableDefinition::new(&name);
        let mut out = Vec::new();
        macro_rules! go {
            ($t:expr) => {{
                for k in keys {
                    for v in $t.get(k.as_slice()).map_err(db_err)? {
                        out.push(v.map_err(db_err)?.value());
                    }
                }
            }};
        }
        match self {
            Source::Snap(rt) => match rt.open_multimap_table(def) {
                Ok(t) => go!(t),
                Err(redb::TableError::TableDoesNotExist(_)) => {}
                Err(e) => return Err(db_err(e)),
            },
            Source::Writer(w) => {
                let mut w = w.lock().unwrap();
                let wt = w.wt(info.temp())?;
                let t = wt.open_multimap_table(def).map_err(db_err)?;
                go!(t)
            }
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// The smallest and largest row id of a table.
    pub fn bounds(&self, info: &TableInfo) -> Result<Option<(u64, u64)>> {
        let name = rows_name(info.tid());
        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
        macro_rules! go {
            ($t:expr) => {{
                let first = $t.first().map_err(db_err)?.map(|(k, _)| k.value());
                let last = $t.last().map_err(db_err)?.map(|(k, _)| k.value());
                Ok(first.zip(last))
            }};
        }
        match self {
            Source::Snap(rt) => match rt.open_table(def) {
                Ok(t) => go!(t),
                Err(redb::TableError::TableDoesNotExist(_)) => Ok(None),
                Err(e) => Err(db_err(e)),
            },
            Source::Writer(w) => {
                let mut w = w.lock().unwrap();
                let wt = w.wt(info.temp())?;
                let t = wt.open_table(def).map_err(db_err)?;
                go!(t)
            }
        }
    }

    /// The number of rows of a table.
    pub fn count(&self, info: &TableInfo) -> Result<u64> {
        let name = rows_name(info.tid());
        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
        match self {
            Source::Snap(rt) => match rt.open_table(def) {
                Ok(t) => t.len().map_err(db_err),
                Err(redb::TableError::TableDoesNotExist(_)) => Ok(0),
                Err(e) => Err(db_err(e)),
            },
            Source::Writer(w) => {
                let mut w = w.lock().unwrap();
                let wt = w.wt(info.temp())?;
                let t = wt.open_table(def).map_err(db_err)?;
                t.len().map_err(db_err)
            }
        }
    }
}

/// What a statement reads: the catalog, the snapshots of both databases, and the writes of
/// its transaction, if it has written.
#[derive(Clone)]
pub struct ReadView {
    pub catalog: Arc<Catalog>,
    pub main: Arc<ReadTransaction>,
    pub temp: Arc<ReadTransaction>,
    pub writer: Option<Arc<Mutex<TxWriter>>>,
    pub metrics: Arc<Metrics>,
    /// How each scan of the statement reads its table (for EXPLAIN).
    pub scans: Arc<Mutex<Vec<String>>>,
}

impl std::fmt::Debug for ReadView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadView")
    }
}

impl ReadView {
    /// Where to read a table: through the transaction's writer if it wrote the table, else from the snapshot.
    pub fn source(&self, info: &TableInfo) -> Source {
        if let Some(w) = &self.writer {
            if w.lock().unwrap().written.contains(&info.tid()) {
                return Source::Writer(w.clone());
            }
        }
        Source::Snap(if info.temp() { self.temp.clone() } else { self.main.clone() })
    }
}

// ------------------------------------------------------------------ DataFusion: catalog and table

/// A table as DataFusion sees it. It holds no rows: a scan reads them through the statement's [`ReadView`].
#[derive(Debug)]
pub struct DepTable {
    pub name: String,
    pub tid: u64,
    schema: SchemaRef,
    temporary: bool,
}

impl DepTable {
    pub fn new(meta: &TableMeta, with_rowid: bool) -> Self {
        let mut fields: Vec<Field> = meta.physical_schema().fields().iter().map(|f| f.as_ref().clone()).collect();
        if with_rowid {
            fields.push(Field::new(ROWID, DataType::UInt64, false));
        }
        DepTable { name: meta.name.clone(), tid: meta.tid, schema: Arc::new(Schema::new(fields)), temporary: meta.temporary }
    }

    /// A filter the scan can answer from an index: (column, literals).
    fn lookup_of<'e>(&self, e: &'e Expr) -> Option<(String, Vec<&'e ScalarValue>)> {
        match e {
            Expr::BinaryExpr(BinaryExpr { left, op: Operator::Eq, right }) => match (left.as_ref(), right.as_ref()) {
                (Expr::Column(c), Expr::Literal(v, _)) | (Expr::Literal(v, _), Expr::Column(c)) => Some((c.name.clone(), vec![v])),
                _ => None,
            },
            // DataFusion turns a short IN list into ORed equalities.
            Expr::BinaryExpr(BinaryExpr { left, op: Operator::Or, right }) => {
                let (c1, mut v1) = self.lookup_of(left)?;
                let (c2, v2) = self.lookup_of(right)?;
                if c1 != c2 || v1.len() + v2.len() > MAX_LOOKUP_KEYS {
                    return None;
                }
                v1.extend(v2);
                Some((c1, v1))
            }
            Expr::InList(l) if !l.negated && l.list.len() <= MAX_LOOKUP_KEYS => {
                let Expr::Column(c) = l.expr.as_ref() else { return None };
                let mut vals = Vec::new();
                for x in &l.list {
                    match x {
                        Expr::Literal(v, _) => vals.push(v),
                        _ => return None,
                    }
                }
                Some((c.name.clone(), vals))
            }
            _ => None,
        }
    }
}

#[async_trait]
impl TableProvider for DepTable {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> TableType {
        if self.temporary {
            TableType::Temporary
        } else {
            TableType::Base
        }
    }

    fn supports_filters_pushdown(&self, filters: &[&Expr]) -> DFResult<Vec<TableProviderFilterPushDown>> {
        // Inexact: DataFusion still applies the filter to the rows the index returns.
        Ok(filters
            .iter()
            .map(|f| if self.lookup_of(f).is_some() { TableProviderFilterPushDown::Inexact } else { TableProviderFilterPushDown::Unsupported })
            .collect())
    }

    async fn scan(&self, state: &dyn Session, projection: Option<&Vec<usize>>, filters: &[Expr], limit: Option<usize>) -> DFResult<Arc<dyn ExecutionPlan>> {
        let view = state
            .config()
            .get_extension::<ReadView>()
            .ok_or_else(|| df(DepError::sql("XX000", format!("internal: {} was read outside a statement", self.name))))?;
        let meta = match view.catalog.tables.get(&self.name) {
            Some(m) if m.tid == self.tid => m,
            Some(_) => {
                return Err(df(DepError::sql("55000", format!("relation \"{}\" was dropped and created again; re-create the functions that use it", self.name))));
            }
            None => return Err(df(DepError::sql("42P01", format!("relation \"{}\" does not exist", self.name)))),
        };
        let info = TableInfo::new(meta);
        let all: Vec<usize> = (0..self.schema.fields().len()).collect();
        let proj = projection.cloned().unwrap_or(all);
        let width = info.types.len();
        let out: Vec<usize> = proj.iter().map(|p| if *p >= width { usize::MAX } else { *p }).collect();
        let schema = Arc::new(self.schema.project(&proj)?);
        let source = view.source(&info);
        let batch = state.config().batch_size();
        // An index lookup, if a filter allows one: the one with the fewest keys.
        let mut best: Option<(String, Vec<Vec<u8>>)> = None;
        for f in filters {
            let Some((col, vals)) = self.lookup_of(f) else { continue };
            let Some(ix) = meta.single_index(&col).or_else(|| meta.indexes.iter().find(|i| i.columns.first() == Some(&col) && i.columns.len() == 1)) else { continue };
            let Some(pos) = info.schema.index_of(&col).ok() else { continue };
            let t = &info.types[pos];
            let mut keys = Vec::new();
            let mut ok = true;
            for v in vals {
                match scalar_key(t, v) {
                    Some(Some(k)) => keys.push(k),
                    Some(None) => {}
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && best.as_ref().map(|(_, k)| keys.len() < k.len()).unwrap_or(true) {
                best = Some((ix.name.clone(), keys));
            }
        }
        let parts: Vec<Arc<dyn PartitionStream>> = match best {
            Some((index, keys)) => {
                Metrics::inc(&view.metrics.index_lookups, 1);
                view.scans.lock().unwrap().push(format!("{}: index {index}, {} key(s)", self.name, keys.len()));
                vec![Arc::new(ScanPart { schema: schema.clone(), info: info.clone(), out: out.clone(), source, plan: PartPlan::Lookup { index, keys }, limit, batch, metrics: view.metrics.clone() })]
            }
            None => {
                Metrics::inc(&view.metrics.full_scans, 1);
                view.scans.lock().unwrap().push(format!("{}: every row", self.name));
                let bounds = source.bounds(&info).map_err(df)?;
                let n = state.config().target_partitions().max(1) as u64;
                match bounds {
                    None => vec![Arc::new(ScanPart { schema: schema.clone(), info, out, source, plan: PartPlan::Empty, limit, batch, metrics: view.metrics.clone() })],
                    Some((lo, hi)) => {
                        let span = hi - lo + 1;
                        let parts = if span < 4 * batch as u64 { 1 } else { n.min(span / batch as u64).max(1) };
                        let step = span.div_ceil(parts);
                        (0..parts)
                            .map(|i| {
                                let a = lo + i * step;
                                let b = (a + step - 1).min(hi);
                                Arc::new(ScanPart {
                                    schema: schema.clone(),
                                    info: info.clone(),
                                    out: out.clone(),
                                    source: source.clone(),
                                    plan: PartPlan::Range(a, b),
                                    limit,
                                    batch,
                                    metrics: view.metrics.clone(),
                                }) as Arc<dyn PartitionStream>
                            })
                            .collect()
                    }
                }
            }
        };
        Ok(Arc::new(StreamingTableExec::try_new(schema, parts, None, vec![], false, limit)?))
    }
}

enum PartPlan {
    Empty,
    Range(u64, u64),
    Lookup { index: String, keys: Vec<Vec<u8>> },
}

struct ScanPart {
    schema: SchemaRef,
    info: Arc<TableInfo>,
    out: Vec<usize>,
    source: Source,
    plan: PartPlan,
    limit: Option<usize>,
    batch: usize,
    metrics: Arc<Metrics>,
}

impl std::fmt::Debug for ScanPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.plan {
            PartPlan::Empty => write!(f, "DepScan({}, empty)", self.info.meta.name),
            PartPlan::Range(a, b) => write!(f, "DepScan({}, rows {a}..={b})", self.info.meta.name),
            PartPlan::Lookup { index, keys } => write!(f, "DepScan({}, index {index}, {} keys)", self.info.meta.name, keys.len()),
        }
    }
}

impl PartitionStream for ScanPart {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    fn execute(&self, _ctx: Arc<TaskContext>) -> SendableRecordBatchStream {
        let mut builder = RecordBatchReceiverStreamBuilder::new(self.schema.clone(), 2);
        let tx = builder.tx();
        let info = self.info.clone();
        let out = self.out.clone();
        let source = self.source.clone();
        let schema = self.schema.clone();
        let limit = self.limit.unwrap_or(usize::MAX);
        let batch = self.batch.max(1);
        let metrics = self.metrics.clone();
        let plan = match &self.plan {
            PartPlan::Empty => PartPlan::Empty,
            PartPlan::Range(a, b) => PartPlan::Range(*a, *b),
            PartPlan::Lookup { index, keys } => PartPlan::Lookup { index: index.clone(), keys: keys.clone() },
        };
        builder.spawn_blocking(move || {
            let mut bb = BatchBuilder::new(&info, &out, schema, batch.min(limit)).map_err(df)?;
            let mut sent = 0usize;
            let emit = |bb: &mut BatchBuilder, sent: &mut usize| -> DFResult<bool> {
                let n = bb.len();
                let b = bb.finish().map_err(df)?;
                *sent += n;
                Metrics::inc(&metrics.rows_read, n as u64);
                Ok(tx.blocking_send(Ok(b)).is_ok())
            };
            match plan {
                PartPlan::Empty => {}
                PartPlan::Range(lo, hi) => {
                    let mut from = lo;
                    loop {
                        let want = batch.min(limit.saturating_sub(sent));
                        if want == 0 {
                            break;
                        }
                        let next = source.rows(&info, from, hi, want, &mut |r, v| bb.push(r, v)).map_err(df)?;
                        if !bb.is_empty() && !emit(&mut bb, &mut sent)? {
                            return Ok(());
                        }
                        match next {
                            Some(n) => from = n,
                            None => break,
                        }
                    }
                }
                PartPlan::Lookup { index, keys } => {
                    let ids = source.lookup(&info, &index, &keys).map_err(df)?;
                    for chunk in ids.chunks(batch) {
                        if sent >= limit {
                            break;
                        }
                        source.get_many(&info, chunk, &mut |r, v| bb.push(r, v)).map_err(df)?;
                        if !bb.is_empty() && !emit(&mut bb, &mut sent)? {
                            return Ok(());
                        }
                    }
                }
            }
            Ok(())
        });
        builder.build()
    }
}

/// The tables of a catalog, for DataFusion. `with_rowid` names a table whose schema gets the
/// row id column (the target of an UPDATE or DELETE).
#[derive(Debug)]
pub struct DepSchema {
    pub catalog: Arc<Catalog>,
    pub with_rowid: Option<String>,
}

#[async_trait]
impl SchemaProvider for DepSchema {
    fn table_names(&self) -> Vec<String> {
        self.catalog.tables.keys().cloned().collect()
    }

    async fn table(&self, name: &str) -> DFResult<Option<Arc<dyn TableProvider>>> {
        Ok(self.catalog.tables.get(name).map(|m| Arc::new(DepTable::new(m, self.with_rowid.as_deref() == Some(name))) as Arc<dyn TableProvider>))
    }

    fn table_exist(&self, name: &str) -> bool {
        self.catalog.tables.contains_key(name)
    }
}

#[derive(Debug)]
pub struct DepCatalog(pub Arc<DepSchema>);

impl CatalogProvider for DepCatalog {
    fn schema_names(&self) -> Vec<String> {
        vec!["public".into()]
    }
    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        (name == "public").then(|| self.0.clone() as Arc<dyn SchemaProvider>)
    }
}

#[derive(Debug)]
pub struct DepCatalogList(pub Arc<DepCatalog>);

impl CatalogProviderList for DepCatalogList {
    fn register_catalog(&self, _name: String, _catalog: Arc<dyn CatalogProvider>) -> Option<Arc<dyn CatalogProvider>> {
        None
    }
    fn catalog_names(&self) -> Vec<String> {
        vec!["datafusion".into()]
    }
    fn catalog(&self, name: &str) -> Option<Arc<dyn CatalogProvider>> {
        (name == "datafusion").then(|| self.0.clone() as Arc<dyn CatalogProvider>)
    }
}

// ------------------------------------------------------------------ writing

/// One row change, kept while savepoints are open so ROLLBACK TO can undo it.
struct Undo {
    info: Arc<TableInfo>,
    rowid: u64,
    /// The row before the change (None: it did not exist).
    before: Option<Vec<u8>>,
    /// The row after the change (None: it was deleted).
    after: Option<Vec<u8>>,
}

/// The writes of a transaction: the write lock, and the redb write transactions of the main
/// and the temporary database, opened when first needed.
pub struct TxWriter {
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    main_db: Arc<Database>,
    temp_db: Arc<Database>,
    main: Option<WriteTransaction>,
    temp: Option<WriteTransaction>,
    /// Tables this transaction wrote (or created): it reads them through its write transaction.
    pub written: HashSet<u64>,
    /// Indexes whose data this transaction built: (temporary, tid, index).
    pub built_indexes: HashSet<(bool, u64, String)>,
    /// Tables this transaction created: (temporary, tid).
    pub created: HashSet<(bool, u64)>,
    seq: HashMap<(bool, u64), u64>,
    undo: Vec<Undo>,
    /// Row changes are recorded for undo (a savepoint is open).
    pub recording: bool,
    ended: bool,
}

impl std::fmt::Debug for TxWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TxWriter")
    }
}

fn unique_violation(info: &TableInfo, ix: &IndexInfo, row: &[u8]) -> DepError {
    DepError::sql("23505", format!("duplicate key value violates unique constraint \"{}\": {} already exists.", ix.meta.name, info.describe_key(ix, row)))
}

impl TxWriter {
    pub fn new(guard: tokio::sync::OwnedMutexGuard<()>, main_db: Arc<Database>, temp_db: Arc<Database>) -> Self {
        TxWriter {
            guard: Some(guard),
            main_db,
            temp_db,
            main: None,
            temp: None,
            written: HashSet::new(),
            built_indexes: HashSet::new(),
            created: HashSet::new(),
            seq: HashMap::new(),
            undo: Vec::new(),
            recording: false,
            ended: false,
        }
    }

    /// The write transaction of the main or the temporary database.
    pub fn wt(&mut self, temp: bool) -> Result<&WriteTransaction> {
        if self.ended {
            return Err(DepError::sql("25000", "the transaction has ended"));
        }
        let (slot, db) = if temp { (&mut self.temp, &self.temp_db) } else { (&mut self.main, &self.main_db) };
        if slot.is_none() {
            *slot = Some(db.begin_write().map_err(db_err)?);
        }
        Ok(slot.as_ref().unwrap())
    }

    /// The next row id of a table, and reserves `n` from it.
    pub fn reserve(&mut self, info: &TableInfo, n: u64) -> Result<u64> {
        let key = (info.temp(), info.tid());
        let start = match self.seq.get(&key) {
            Some(s) => *s,
            None => {
                let wt = self.wt(info.temp())?;
                let from_seq = wt.open_table(SEQ).map_err(db_err)?.get(info.tid()).map_err(db_err)?.map(|v| v.value());
                match from_seq {
                    Some(s) => s,
                    None => {
                        let name = rows_name(info.tid());
                        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
                        let last = wt.open_table(def).map_err(db_err)?.last().map_err(db_err)?.map(|(k, _)| k.value());
                        last.map(|l| l + 1).unwrap_or(1)
                    }
                }
            }
        };
        self.seq.insert(key, start + n);
        Ok(start)
    }

    pub fn get(&mut self, info: &TableInfo, rowid: u64) -> Result<Option<Vec<u8>>> {
        let name = rows_name(info.tid());
        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
        let wt = self.wt(info.temp())?;
        let t = wt.open_table(def).map_err(db_err)?;
        let v = t.get(rowid).map_err(db_err)?.map(|v| v.value().to_vec());
        Ok(v)
    }

    /// Writes a row (new, or replacing `before`), keeping the indexes up to date and checking unique keys.
    pub fn put(&mut self, info: &Arc<TableInfo>, rowid: u64, row: &[u8], before: Option<&[u8]>) -> Result<()> {
        self.apply(info, rowid, Some(row), before, true)
    }

    pub fn delete(&mut self, info: &Arc<TableInfo>, rowid: u64, before: &[u8]) -> Result<()> {
        self.apply(info, rowid, None, Some(before), true)
    }

    fn apply(&mut self, info: &Arc<TableInfo>, rowid: u64, row: Option<&[u8]>, before: Option<&[u8]>, check: bool) -> Result<()> {
        let temp = info.temp();
        // The index keys that change.
        let mut changes = Vec::new();
        for ix in &info.indexes {
            let old = match before {
                Some(b) => info.key(ix, b)?,
                None => None,
            };
            let new = match row {
                Some(r) => info.key(ix, r)?,
                None => None,
            };
            if old != new {
                changes.push((ix, old, new));
            }
        }
        let wt = self.wt(temp)?;
        // Every unique key is checked before anything is written, so a row that breaks one key
        // leaves no trace in the others.
        if check {
            for (ix, _, new) in &changes {
                let (Some(k), true) = (new, ix.meta.unique && ix.meta.enforced) else { continue };
                let name = index_name(info.tid(), &ix.meta.name);
                let def: MultimapTableDefinition<&[u8], u64> = MultimapTableDefinition::new(&name);
                let t = wt.open_multimap_table(def).map_err(db_err)?;
                for v in t.get(k.as_slice()).map_err(db_err)? {
                    if v.map_err(db_err)?.value() != rowid {
                        return Err(unique_violation(info, ix, row.unwrap()));
                    }
                }
            }
        }
        for (ix, old, new) in &changes {
            let name = index_name(info.tid(), &ix.meta.name);
            let def: MultimapTableDefinition<&[u8], u64> = MultimapTableDefinition::new(&name);
            let mut t = wt.open_multimap_table(def).map_err(db_err)?;
            if let Some(k) = old {
                t.remove(k.as_slice(), rowid).map_err(db_err)?;
            }
            if let Some(k) = new {
                t.insert(k.as_slice(), rowid).map_err(db_err)?;
            }
        }
        {
            let name = rows_name(info.tid());
            let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
            let mut t = wt.open_table(def).map_err(db_err)?;
            match row {
                Some(r) => {
                    t.insert(rowid, r).map_err(db_err)?;
                }
                None => {
                    t.remove(rowid).map_err(db_err)?;
                }
            }
        }
        if self.recording {
            self.undo.push(Undo { info: info.clone(), rowid, before: before.map(|b| b.to_vec()), after: row.map(|r| r.to_vec()) });
        }
        Ok(())
    }

    /// Builds an index from the rows of its table. With `strict`, duplicate keys in a unique
    /// index are an error; otherwise the number of duplicate keys is returned.
    pub fn build_index(&mut self, info: &TableInfo, ix: &IndexInfo, strict: bool) -> Result<u64> {
        let temp = info.temp();
        let dups = self.build_index_inner(info, ix, strict)?;
        self.built_indexes.insert((temp, info.tid(), ix.meta.name.clone()));
        Ok(dups)
    }

    fn build_index_inner(&mut self, info: &TableInfo, ix: &IndexInfo, strict: bool) -> Result<u64> {
        let wt = self.wt(info.temp())?;
        let iname = index_name(info.tid(), &ix.meta.name);
        let idef: MultimapTableDefinition<&[u8], u64> = MultimapTableDefinition::new(&iname);
        let _ = wt.delete_multimap_table(idef);
        let mut it = wt.open_multimap_table(idef).map_err(db_err)?;
        let name = rows_name(info.tid());
        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
        let rows = wt.open_table(def).map_err(db_err)?;
        let mut dups = 0u64;
        for item in rows.iter().map_err(db_err)? {
            let (k, v) = item.map_err(db_err)?;
            let Some(key) = info.key(ix, v.value())? else { continue };
            if ix.meta.unique {
                let existing = it.get(key.as_slice()).map_err(db_err)?.count();
                if existing > 0 {
                    if strict {
                        return Err(unique_violation(info, ix, v.value()).with_hint("The table has rows with equal keys; remove the duplicates first."));
                    }
                    dups += 1;
                }
            }
            it.insert(key.as_slice(), k.value()).map_err(db_err)?;
        }
        Ok(dups)
    }

    /// Calls `f` for every row of a table, in batches read through the write transaction.
    pub fn for_each(&mut self, info: &TableInfo, f: &mut dyn FnMut(&mut TxWriter, Vec<(u64, Vec<u8>)>) -> Result<()>) -> Result<()> {
        let mut from = 0u64;
        loop {
            let chunk: Vec<(u64, Vec<u8>)> = {
                let name = rows_name(info.tid());
                let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
                let wt = self.wt(info.temp())?;
                let t = wt.open_table(def).map_err(db_err)?;
                let mut v = Vec::new();
                for item in t.range(from..).map_err(db_err)? {
                    let (k, r) = item.map_err(db_err)?;
                    v.push((k.value(), r.value().to_vec()));
                    if v.len() >= 8192 {
                        break;
                    }
                }
                v
            };
            let Some(last) = chunk.last().map(|(k, _)| *k) else { return Ok(()) };
            f(self, chunk)?;
            from = last + 1;
        }
    }

    /// Replaces a row without touching indexes (for key rotation, which rebuilds them after).
    pub fn put_raw(&mut self, info: &TableInfo, rowid: u64, row: &[u8]) -> Result<()> {
        let name = rows_name(info.tid());
        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
        let wt = self.wt(info.temp())?;
        wt.open_table(def).map_err(db_err)?.insert(rowid, row).map_err(db_err)?;
        Ok(())
    }

    /// Whether an index holds a key (through this transaction's writes).
    pub fn has_key(&mut self, info: &TableInfo, index: &str, key: &[u8], except: Option<u64>) -> Result<bool> {
        let name = index_name(info.tid(), index);
        let def: MultimapTableDefinition<&[u8], u64> = MultimapTableDefinition::new(&name);
        let wt = self.wt(info.temp())?;
        let t = wt.open_multimap_table(def).map_err(db_err)?;
        for v in t.get(key).map_err(db_err)? {
            if Some(v.map_err(db_err)?.value()) != except {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The names of the stored indexes of a table.
    pub fn stored_indexes(&mut self, temp: bool, tid: u64) -> Result<Vec<String>> {
        let prefix = format!("idx/{tid}/");
        let wt = self.wt(temp)?;
        let mut out = Vec::new();
        for h in wt.list_multimap_tables().map_err(db_err)? {
            if let Some(n) = h.name().strip_prefix(&prefix) {
                out.push(n.to_string());
            }
        }
        Ok(out)
    }

    /// Deletes the stored rows and indexes of a table.
    pub fn drop_table_data(&mut self, temp: bool, tid: u64) -> Result<()> {
        for i in self.stored_indexes(temp, tid)? {
            self.drop_index_data(temp, tid, &i)?;
        }
        let wt = self.wt(temp)?;
        let name = rows_name(tid);
        let def: TableDefinition<u64, &[u8]> = TableDefinition::new(&name);
        wt.delete_table(def).map_err(db_err)?;
        wt.open_table(SEQ).map_err(db_err)?.remove(tid).map_err(db_err)?;
        self.seq.remove(&(temp, tid));
        Ok(())
    }

    /// Deletes a stored index.
    pub fn drop_index_data(&mut self, temp: bool, tid: u64, index: &str) -> Result<()> {
        let wt = self.wt(temp)?;
        let n = index_name(tid, index);
        let d: MultimapTableDefinition<&[u8], u64> = MultimapTableDefinition::new(&n);
        wt.delete_multimap_table(d).map_err(db_err)?;
        Ok(())
    }

    /// Sets the next row id of a table (imports).
    pub fn set_seq(&mut self, info: &TableInfo, next: u64) {
        self.seq.insert((info.temp(), info.tid()), next);
    }

    /// Starts recording row changes for undo; returns the position to roll back to.
    pub fn mark(&mut self) -> usize {
        self.recording = true;
        self.undo.len()
    }

    /// Undoes every row change after `mark`, newest first.
    pub fn undo_to(&mut self, mark: usize) -> Result<()> {
        let was = self.recording;
        self.recording = false;
        while self.undo.len() > mark {
            let u = self.undo.pop().unwrap();
            self.apply(&u.info, u.rowid, u.before.as_deref(), u.after.as_deref(), false)?;
        }
        self.recording = was;
        Ok(())
    }

    /// Stops recording when no savepoint is left.
    pub fn release_marks(&mut self) {
        self.recording = false;
        self.undo.clear();
    }

    /// Writes the row id sequences and commits both databases. Blocking. The write lock is
    /// kept until [`Self::release`], so the caller can publish the new catalog first.
    pub fn commit(&mut self) -> Result<()> {
        if self.ended {
            return Err(DepError::sql("25000", "the transaction has ended"));
        }
        self.ended = true;
        for temp in [false, true] {
            let seq: Vec<(u64, u64)> = self.seq.iter().filter(|((t, _), _)| *t == temp).map(|((_, tid), s)| (*tid, *s)).collect();
            let slot = if temp { &mut self.temp } else { &mut self.main };
            let Some(wt) = slot.as_ref() else { continue };
            let mut st = wt.open_table(SEQ).map_err(db_err)?;
            for (tid, s) in seq {
                st.insert(tid, s).map_err(db_err)?;
            }
        }
        if let Some(w) = self.main.take() {
            w.commit().map_err(|e| DepError::sql("58030", format!("could not commit to the data file: {e}")))?;
        }
        if let Some(w) = self.temp.take() {
            w.commit().map_err(db_err)?;
        }
        Ok(())
    }

    /// Releases the write lock.
    pub fn release(&mut self) {
        self.guard.take();
    }

    /// Discards everything (the write lock is released when the writer is dropped).
    pub fn abort(&mut self) {
        self.ended = true;
        if let Some(w) = self.main.take() {
            let _ = w.abort();
        }
        if let Some(w) = self.temp.take() {
            let _ = w.abort();
        }
        self.guard.take();
    }

    /// Writes a key of the main database's meta table (the catalog, the key fingerprint).
    pub fn put_meta(&mut self, key: &str, value: &[u8]) -> Result<()> {
        let wt = self.wt(false)?;
        wt.open_table(crate::persist::META).map_err(db_err)?.insert(key, value).map_err(db_err)?;
        Ok(())
    }
}

impl Drop for TxWriter {
    fn drop(&mut self) {
        if !self.ended {
            self.abort();
        }
    }
}

/// Runs blocking work (a commit, a batch of row writes) without stalling the async runtime:
/// in place on a multi-threaded runtime, which moves its other tasks to other threads.
pub fn blocking<R>(f: impl FnOnce() -> R) -> R {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// The physical column positions of a batch's columns that must not be NULL; the first NULL found, as an error.
pub fn check_not_null(info: &TableInfo, batch: &RecordBatch) -> Result<()> {
    for (pos, name) in &info.not_null {
        let a = batch.column(*pos);
        if a.null_count() > 0 {
            return Err(DepError::sql("23502", format!("null value in column \"{name}\" of relation \"{}\" violates not-null constraint", info.meta.name)));
        }
    }
    Ok(())
}

/// Builds a batch of a table's physical schema from encoded rows (for key rotation and tests).
pub fn decode_rows(info: &TableInfo, rows: &[(u64, Vec<u8>)]) -> Result<RecordBatch> {
    let out: Vec<usize> = (0..info.types.len()).collect();
    let mut b = BatchBuilder::new(info, &out, info.schema.clone(), rows.len().max(1))?;
    for (r, v) in rows {
        b.push(*r, v)?;
    }
    b.finish()
}

/// The name of the ciphertext column of a protected column.
pub fn ct_name(column: &str) -> String {
    format!("{column}{CT_SUFFIX}")
}
