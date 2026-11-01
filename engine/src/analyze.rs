//! The DEP analyzer: a pass over DataFusion's logical plan that runs before
//! DataFusion's own analyzer and optimizer.
//!
//! It follows every output column of every plan node and gives it a kind:
//! ordinary, PROTECTED (with its match domain), the hidden ciphertext of a
//! protected column, or DISCLOSED (released by a declared function). With
//! those kinds it applies the rules of the proposal and rewrites the plan:
//!
//! - the only operation on a protected value is full-value equality, against a
//!   value or another protected column of the same match domain; the value side
//!   is replaced by its keyed index entry, `dep_index(domain, value)`;
//! - on a MASKED_READ connection, protected values are never matched;
//! - every join on protected columns must be anchored to one protected value by
//!   a top-level equality, unless it runs in a function declared LINKS;
//! - REVEAL is admitted only in a function declared DISCLOSES for the value's
//!   match domain, where it becomes `dep_decrypt(domain, ciphertext)`;
//! - disclosed values may be returned, and nothing else (the terminal rule);
//! - a protected column in the final output becomes NULL.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use datafusion::arrow::datatypes::DataType;
use datafusion::catalog::default_table_source::source_as_provider;
use datafusion::common::tree_node::{Transformed, TreeNode};
use datafusion::common::{Column, DFSchema, ScalarValue, TableReference};
use datafusion::logical_expr::expr::{Alias, Exists, InList, InSubquery};
use datafusion::logical_expr::utils::split_conjunction;
use datafusion::logical_expr::{cast, lit, BinaryExpr, Distinct, Expr, Join, LogicalPlan, Operator, Projection, ScalarUDF, Subquery, SubqueryAlias};

use crate::catalog::{Catalog, FunctionMeta, PathMode, CT_SUFFIX};
use crate::error::{DepError, Result};
use crate::functions::DepFunctionTable;

#[derive(Clone, Debug, PartialEq)]
pub enum Taint {
    Ord,
    Prot {
        domain: String,
        desc: String,
        anchored: bool,
        has_ct: bool,
    },
    /// The hidden ciphertext column of a protected column.
    Ct,
    Disc {
        domain: String,
        desc: String,
    },
}

impl Taint {
    fn is_prot(&self) -> bool {
        matches!(self, Taint::Prot { .. })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Place {
    Filter,
    JoinCond,
    Project,
    GroupBy,
    Aggregate,
    Sort,
    Window,
}

impl Place {
    fn desc(&self) -> &'static str {
        match self {
            Place::Filter => "a filter",
            Place::JoinCond => "a join condition",
            Place::Project => "an expression",
            Place::GroupBy => "GROUP BY",
            Place::Aggregate => "an aggregate",
            Place::Sort => "ORDER BY",
            Place::Window => "a window function",
        }
    }
}

#[derive(Clone, Debug)]
struct Edge {
    a: String,
    b: String,
    da: String,
    db: String,
    domain: String,
}

/// The match graph of one query block: protected columns joined to each other,
/// and the columns anchored to a single value.
#[derive(Clone, Debug, Default)]
struct Block {
    edges: Vec<Edge>,
    anchors: HashSet<String>,
}

impl Block {
    fn merge(&mut self, o: Block) {
        self.edges.extend(o.edges);
        self.anchors.extend(o.anchors);
    }
    fn rename(&mut self, map: &HashMap<String, String>) {
        let r = |k: &String| map.get(k).cloned().unwrap_or_else(|| k.clone());
        for e in &mut self.edges {
            e.a = r(&e.a);
            e.b = r(&e.b);
        }
        self.anchors = self.anchors.iter().map(r).collect();
    }
    fn components(&self) -> HashMap<String, String> {
        let mut parent: HashMap<String, String> = HashMap::new();
        fn find(p: &mut HashMap<String, String>, k: &str) -> String {
            let mut cur = k.to_string();
            loop {
                let next = p.get(&cur).cloned().unwrap_or_else(|| cur.clone());
                if next == cur {
                    return cur;
                }
                cur = next;
            }
        }
        for e in &self.edges {
            let ra = find(&mut parent, &e.a);
            let rb = find(&mut parent, &e.b);
            if ra != rb {
                parent.insert(ra, rb);
            }
        }
        let keys: Vec<String> =
            parent.keys().cloned().chain(self.edges.iter().flat_map(|e| [e.a.clone(), e.b.clone()])).chain(self.anchors.iter().cloned()).collect();
        keys.into_iter().map(|k| (k.clone(), find(&mut parent, &k))).collect()
    }
    fn anchored_roots(&self) -> HashSet<String> {
        let comp = self.components();
        self.anchors.iter().map(|a| comp.get(a).cloned().unwrap_or_else(|| a.clone())).collect()
    }
    fn is_anchored(&self, key: &str) -> bool {
        let comp = self.components();
        let root = comp.get(key).cloned().unwrap_or_else(|| key.to_string());
        self.anchored_roots().contains(&root)
    }
}

pub struct Node {
    pub plan: LogicalPlan,
    pub taints: Vec<Taint>,
    block: Block,
}

#[derive(Clone, Debug, Default)]
pub struct Region {
    pub name: String,
    pub discloses: Vec<String>,
    pub links: Vec<String>,
}

struct Scope {
    schema: DFSchema,
    taints: Vec<Taint>,
}

pub struct Analyzer<'a> {
    pub path: PathMode,
    pub region: Option<Region>,
    pub catalog: &'a Catalog,
    pub index_udf: Arc<ScalarUDF>,
    pub decrypt_udf: Arc<ScalarUDF>,
    /// Set when a protected column was matched against a function parameter.
    pub param_match: bool,
    outer: Vec<Scope>,
    /// Matches between protected columns found while rewriting expressions, waiting
    /// to join the match graph of the block they belong to.
    pending: Vec<Edge>,
    pending_anchors: HashSet<String>,
    seq: usize,
    /// Notes about how the DEP rules applied to the statement, for the client to show.
    pub notes: std::cell::RefCell<Vec<String>>,
    /// Uses of protected data in the top-level statement, for the audit log and the rate limits.
    pub events: std::cell::RefCell<Vec<crate::audit::Event>>,
}

fn key_of(schema: &DFSchema, i: usize) -> String {
    match schema.qualified_field(i) {
        (Some(q), f) => format!("{q}.{}", f.name()),
        (None, f) => f.name().clone(),
    }
}

fn is_constant(e: &Expr) -> bool {
    let mut ok = true;
    e.apply(|x| {
        if matches!(
            x,
            Expr::Column(_)
                | Expr::OuterReferenceColumn(..)
                | Expr::ScalarSubquery(_)
                | Expr::Exists(_)
                | Expr::InSubquery(_)
                | Expr::AggregateFunction(_)
                | Expr::WindowFunction(_)
        ) {
            ok = false;
        }
        Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
    })
    .ok();
    ok
}

fn has_placeholder(e: &Expr) -> bool {
    let mut found = false;
    e.apply(|x| {
        if matches!(x, Expr::Placeholder(_)) {
            found = true;
        }
        Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
    })
    .ok();
    found
}

fn op_name(e: &Expr) -> String {
    match e {
        Expr::BinaryExpr(b) => format!("Operator {}", b.op),
        Expr::Like(l) => if l.case_insensitive { "ILIKE" } else { "LIKE" }.to_string(),
        Expr::SimilarTo(_) => "SIMILAR TO".into(),
        Expr::ScalarFunction(f) => format!("Function {}()", f.name()),
        Expr::AggregateFunction(f) => format!("Aggregate {}()", f.func.name()),
        Expr::WindowFunction(_) => "A window function".into(),
        Expr::Case(_) => "CASE".into(),
        Expr::Between(_) => "BETWEEN".into(),
        Expr::Not(_) => "NOT".into(),
        Expr::Negative(_) => "Negation".into(),
        Expr::IsTrue(_) | Expr::IsFalse(_) | Expr::IsUnknown(_) | Expr::IsNotTrue(_) | Expr::IsNotFalse(_) | Expr::IsNotUnknown(_) => "A boolean test".into(),
        _ => "This operation".into(),
    }
}

fn sql_type_name(t: &DataType) -> String {
    match t {
        DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8 => "TEXT".into(),
        DataType::Int64 => "BIGINT".into(),
        DataType::Int32 => "INTEGER".into(),
        DataType::Int16 => "SMALLINT".into(),
        DataType::Float64 => "DOUBLE PRECISION".into(),
        DataType::Boolean => "BOOLEAN".into(),
        DataType::Date32 => "DATE".into(),
        DataType::Timestamp(..) => "TIMESTAMP".into(),
        other => other.to_string().to_uppercase(),
    }
}

static OPAQUE: std::sync::LazyLock<Arc<ScalarUDF>> = std::sync::LazyLock::new(crate::udf::Opaque::udf);

fn strip_alias(e: &Expr) -> &Expr {
    match e {
        Expr::Alias(a) => strip_alias(&a.expr),
        other => other,
    }
}

fn companion(c: &Column) -> Column {
    Column::new(c.relation.clone(), format!("{}{CT_SUFFIX}", c.name))
}

impl<'a> Analyzer<'a> {
    pub fn new(path: PathMode, region: Option<Region>, catalog: &'a Catalog, index_udf: Arc<ScalarUDF>, decrypt_udf: Arc<ScalarUDF>) -> Self {
        Analyzer { path, region, catalog, index_udf, decrypt_udf, param_match: false, outer: vec![], pending: vec![], pending_anchors: HashSet::new(), seq: 0, notes: Default::default(), events: Default::default() }
    }

    /// Records a note for the top-level statement (not for function bodies).
    fn note(&self, text: String) {
        if self.region.is_some() {
            return;
        }
        let mut n = self.notes.borrow_mut();
        if !n.contains(&text) {
            n.push(text);
        }
    }

    fn event(&self, kind: &'static str, object: &str, domain: Option<&str>, count: u32) {
        if self.region.is_none() {
            self.events.borrow_mut().push(crate::audit::Event { kind, object: object.to_string(), domain: domain.map(str::to_string), count });
        }
    }

    pub fn take_events(&self) -> Vec<crate::audit::Event> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    pub fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut *self.notes.borrow_mut())
    }

    fn links(&self, domain: &str) -> bool {
        self.region.as_ref().map(|r| r.links.iter().any(|d| d == domain)).unwrap_or(false)
    }

    fn check_match_allowed(&self) -> Result<()> {
        if self.region.is_none() && self.path == PathMode::MaskedRead {
            return Err(DepError::masked("Matching PROTECTED values is"));
        }
        Ok(())
    }

    fn index_expr(&mut self, domain: &str, value: Expr) -> Expr {
        if has_placeholder(&value) {
            self.param_match = true;
        }
        self.index_udf.call(vec![lit(domain), cast(value, DataType::Utf8)])
    }

    // ------------------------------------------------------------------ plans

    pub fn analyze(&mut self, plan: &LogicalPlan) -> Result<Node> {
        match plan {
            LogicalPlan::TableScan(ts) => self.table_scan(plan, ts),
            LogicalPlan::Filter(f) => {
                let input = self.analyze(&f.input)?;
                let mut block = input.block;
                let schema = input.plan.schema().clone();
                self.scan_conjuncts(&f.predicate, &schema, &input.taints, &mut block);
                let (pred, _) = self.rw(&f.predicate, &schema, &input.taints, Place::Filter)?;
                self.drain(&mut block);
                let plan = plan.with_new_exprs(vec![pred], vec![input.plan])?;
                Ok(Node { plan, taints: input.taints, block })
            }
            LogicalPlan::Join(j) => self.join(j),
            LogicalPlan::Projection(p) => self.projection(p),
            LogicalPlan::SubqueryAlias(sa) => {
                let input = self.analyze(&sa.input)?;
                let old = input.plan.schema().clone();
                let new_plan = LogicalPlan::SubqueryAlias(SubqueryAlias::try_new(Arc::new(input.plan), sa.alias.clone())?);
                let new = new_plan.schema().clone();
                let map: HashMap<String, String> = (0..old.fields().len()).map(|i| (key_of(&old, i), key_of(&new, i))).collect();
                let mut block = input.block;
                block.rename(&map);
                Ok(Node { plan: new_plan, taints: input.taints, block })
            }
            LogicalPlan::Aggregate(a) => {
                let mut input = self.analyze(&a.input)?;
                let schema = input.plan.schema().clone();
                let mut exprs = Vec::new();
                let mut taints = Vec::new();
                for g in &a.group_expr {
                    let (e, t) = self.rw(g, &schema, &input.taints, Place::GroupBy)?;
                    if let Taint::Prot { domain, desc, .. } = &t {
                        if !self.links(domain) {
                            return Err(DepError::dep(
                                "DEP_UNANCHORED_MATCH",
                                format!("GROUP BY on PROTECTED column {desc} links every row that shares a value."),
                                Some(&format!("Group by ordinary columns, or perform the grouping inside a function declared LINKS {domain}.")),
                            ));
                        }
                    }
                    taints.push(match t {
                        Taint::Prot { domain, desc, .. } => Taint::Prot { domain, desc, anchored: false, has_ct: false },
                        other => other,
                    });
                    exprs.push(e);
                }
                for ag in &a.aggr_expr {
                    let (e, _) = self.rw(ag, &schema, &input.taints, Place::Aggregate)?;
                    exprs.push(e);
                    taints.push(Taint::Ord);
                }
                self.drain(&mut input.block);
                self.close(&input.block)?;
                let plan = plan.with_new_exprs(exprs, vec![input.plan])?;
                Ok(Node { plan, taints, block: Block::default() })
            }
            LogicalPlan::Sort(s) => {
                let mut input = self.analyze(&s.input)?;
                let schema = input.plan.schema().clone();
                let mut exprs = Vec::new();
                for se in &s.expr {
                    let (e, t) = self.rw(&se.expr, &schema, &input.taints, Place::Sort)?;
                    if let Taint::Prot { desc, .. } = &t {
                        return Err(DepError::protected_op("ORDER BY", desc));
                    }
                    exprs.push(e);
                }
                self.drain(&mut input.block);
                let plan = plan.with_new_exprs(exprs, vec![input.plan])?;
                Ok(Node { plan, taints: input.taints, block: input.block })
            }
            LogicalPlan::Limit(_) => {
                let input = self.analyze(plan.inputs()[0])?;
                let plan = plan.with_new_exprs(plan.expressions(), vec![input.plan])?;
                Ok(Node { plan, taints: input.taints, block: input.block })
            }
            LogicalPlan::Distinct(Distinct::All(inner)) => {
                let input = self.analyze(inner)?;
                for t in &input.taints {
                    if let Taint::Prot { domain, desc, .. } = t {
                        if !self.links(domain) {
                            return Err(DepError::dep(
                                "DEP_UNANCHORED_MATCH",
                                format!("DISTINCT on PROTECTED column {desc} links every row that shares a value."),
                                Some(&format!("Perform it inside a function declared LINKS {domain}.")),
                            ));
                        }
                    }
                }
                let plan = LogicalPlan::Distinct(Distinct::All(Arc::new(input.plan)));
                Ok(Node { plan, taints: input.taints, block: input.block })
            }
            LogicalPlan::Union(_) => {
                let mut inputs = Vec::new();
                let mut taints: Option<Vec<Taint>> = None;
                for i in plan.inputs() {
                    let n = self.analyze(i)?;
                    self.close(&n.block)?;
                    if n.taints.iter().any(|t| matches!(t, Taint::Prot { .. } | Taint::Ct)) {
                        return Err(DepError::unsupported("UNION over PROTECTED columns is not supported."));
                    }
                    taints = Some(match taints {
                        None => n.taints.clone(),
                        Some(prev) => prev.into_iter().zip(n.taints.iter()).map(|(a, b)| if matches!(a, Taint::Disc { .. }) { a } else { b.clone() }).collect(),
                    });
                    inputs.push(n.plan);
                }
                let plan = plan.with_new_exprs(plan.expressions(), inputs)?;
                Ok(Node { plan, taints: taints.unwrap_or_default(), block: Block::default() })
            }
            LogicalPlan::Window(w) => {
                let mut input = self.analyze(&w.input)?;
                let schema = input.plan.schema().clone();
                let mut exprs = Vec::new();
                for e in &w.window_expr {
                    exprs.push(self.rw(e, &schema, &input.taints, Place::Window)?.0);
                }
                self.drain(&mut input.block);
                let mut taints = input.taints.clone();
                taints.extend(w.window_expr.iter().map(|_| Taint::Ord));
                let plan = plan.with_new_exprs(exprs, vec![input.plan])?;
                Ok(Node { plan, taints, block: input.block })
            }
            LogicalPlan::Values(_) | LogicalPlan::EmptyRelation(_) => {
                Ok(Node { plan: plan.clone(), taints: vec![Taint::Ord; plan.schema().fields().len()], block: Block::default() })
            }
            other => Err(DepError::unsupported(format!("This query form is not supported ({}).", other.display()))),
        }
    }

    fn table_scan(&mut self, plan: &LogicalPlan, ts: &datafusion::logical_expr::TableScan) -> Result<Node> {
        let schema = plan.schema();
        let provider = source_as_provider(&ts.source).ok();
        if let Some(f) = provider.as_ref().and_then(|p| (p.as_ref() as &dyn std::any::Any).downcast_ref::<DepFunctionTable>().map(|t| t.meta.clone())) {
            self.check_function_call(&f)?;
            if !f.discloses.is_empty() {
                self.note(format!("{}() is declared DISCLOSES {}: the columns it declares are returned in plaintext.", f.name, f.discloses.join(", ")));
                self.event("disclose", &format!("{}()", f.name), Some(&f.discloses.join(", ")), 0);
            }
            if !f.links.is_empty() {
                self.note(format!("{}() is declared LINKS {}: it may join protected values without an anchor.", f.name, f.links.join(", ")));
                self.event("link", &format!("{}()", f.name), Some(&f.links.join(", ")), 0);
            }
            let taints = schema
                .fields()
                .iter()
                .map(|fld| match f.disclosed.iter().find(|(n, _)| n == fld.name()) {
                    Some((n, d)) => Taint::Disc { domain: d.clone(), desc: format!("Column {n} of {}()", f.name) },
                    None => Taint::Ord,
                })
                .collect();
            return Ok(Node { plan: plan.clone(), taints, block: Block::default() });
        }
        let table = ts.table_name.table().to_string();
        let meta = self.catalog.tables.get(&table);
        let taints = schema
            .fields()
            .iter()
            .map(|fld| {
                let name = fld.name();
                match meta.and_then(|m| m.column(name)) {
                    Some(c) if c.domain.is_some() => {
                        Taint::Prot { domain: c.domain.clone().unwrap(), desc: format!("{table}.{name}"), anchored: false, has_ct: true }
                    }
                    _ if meta.is_some() && name.ends_with(CT_SUFFIX) => Taint::Ct,
                    _ => Taint::Ord,
                }
            })
            .collect();
        Ok(Node { plan: plan.clone(), taints, block: Block::default() })
    }

    fn check_function_call(&self, f: &FunctionMeta) -> Result<()> {
        if let Some(r) = &self.region {
            let ok = f.discloses.iter().all(|d| r.discloses.contains(d)) && f.links.iter().all(|d| r.links.contains(d));
            if !ok {
                return Err(DepError::dep(
                    "DEP_DECLARED_CALL",
                    format!("Function {} calls {}(), which is not declared for the same purpose and match domain.", r.name, f.name),
                    Some("Inline the logic, or declare both functions for the same purpose."),
                ));
            }
            return Ok(());
        }
        if self.path == PathMode::MaskedRead {
            if !f.discloses.is_empty() {
                return Err(DepError::masked("Disclosing functions are"));
            }
            if !f.masked_ok {
                return Err(DepError::masked("Linking functions that take a protected input are"));
            }
        }
        Ok(())
    }

    fn join(&mut self, j: &Join) -> Result<Node> {
        let l = self.analyze(&j.left)?;
        let r = self.analyze(&j.right)?;
        let mut block = l.block;
        block.merge(r.block);
        let (ls, rs) = (l.plan.schema().clone(), r.plan.schema().clone());
        let mut on = Vec::new();
        for (le, re) in &j.on {
            let (le2, lt) = self.rw(le, &ls, &l.taints, Place::JoinCond)?;
            let (re2, rt) = self.rw(re, &rs, &r.taints, Place::JoinCond)?;
            match (&lt, &rt) {
                (Taint::Prot { domain: da, desc: dsa, .. }, Taint::Prot { domain: db, desc: dsb, .. }) => {
                    if da != db {
                        return Err(Self::domain_mismatch(da, dsa, db, dsb));
                    }
                    self.check_match_allowed()?;
                    if let (Expr::Column(ca), Expr::Column(cb)) = (strip_alias(le), strip_alias(re)) {
                        if let (Some(ia), Some(ib)) = (ls.maybe_index_of_column(ca), rs.maybe_index_of_column(cb)) {
                            block.edges.push(Edge { a: key_of(&ls, ia), b: key_of(&rs, ib), da: dsa.clone(), db: dsb.clone(), domain: da.clone() });
                        }
                    }
                }
                (Taint::Prot { desc, .. }, _) | (_, Taint::Prot { desc, .. }) => {
                    return Err(DepError::protected_op("Comparison with a non-PROTECTED column", desc));
                }
                _ => {}
            }
            on.push((le2, re2));
        }
        let left = Arc::new(l.plan);
        let right = Arc::new(r.plan);
        let probe = Join::try_new(left.clone(), right.clone(), on.clone(), None, j.join_type, j.join_constraint, j.null_equality, j.null_aware)?;
        let jschema = probe.schema.clone();
        let mut taints = Vec::new();
        let n = jschema.fields().len();
        if n == l.taints.len() + r.taints.len() {
            taints.extend(l.taints.iter().cloned());
            taints.extend(r.taints.iter().cloned());
        } else if n >= l.taints.len() && jschema.qualified_field(0) == ls.qualified_field(0) {
            taints.extend(l.taints.iter().cloned());
        } else {
            taints.extend(r.taints.iter().cloned());
        }
        while taints.len() < n {
            taints.push(Taint::Ord);
        }
        let filter = match &j.filter {
            Some(f) => {
                self.scan_conjuncts(f, &jschema, &taints, &mut block);
                let (f2, _) = self.rw(f, &jschema, &taints, Place::JoinCond)?;
                self.drain(&mut block);
                Some(f2)
            }
            None => None,
        };
        let join = Join::try_new(left, right, on, filter, j.join_type, j.join_constraint, j.null_equality, j.null_aware)?;
        Ok(Node { plan: LogicalPlan::Join(join), taints, block })
    }

    fn projection(&mut self, p: &Projection) -> Result<Node> {
        let mut input = self.analyze(&p.input)?;
        let schema = input.plan.schema().clone();
        let mut rewritten = Vec::new();
        for e in &p.expr {
            if let Expr::Column(c) = e {
                if let Some(i) = schema.maybe_index_of_column(c) {
                    if input.taints[i] == Taint::Ct {
                        continue; // a hidden column reached through a wildcard
                    }
                }
            }
            let (e2, t) = self.rw(e, &schema, &input.taints, Place::Project)?;
            let name = e.name_for_alias()?;
            let e2 = if e2.name_for_alias()? != name {
                match e {
                    Expr::Alias(a) => e2.alias_qualified(a.relation.clone(), a.name.clone()),
                    _ => e2.alias(name.clone()),
                }
            } else {
                e2
            };
            rewritten.push((e, e2, t));
        }
        self.drain(&mut input.block);
        self.close(&input.block)?;
        let mut exprs = Vec::new();
        let mut taints = Vec::new();
        let mut companions = Vec::new();
        let mut anchored_out = HashSet::new();
        for (e, e2, t) in rewritten {
            let t = match t {
                Taint::Prot { domain, desc, anchored, has_ct } => {
                    let mut anchored = anchored;
                    let mut has_ct = has_ct;
                    match strip_alias(e) {
                        Expr::Column(c) => {
                            if let Some(i) = schema.maybe_index_of_column(c) {
                                anchored = anchored || input.block.is_anchored(&key_of(&schema, i));
                            }
                            if has_ct && schema.maybe_index_of_column(&companion(c)).is_some() {
                                let ce = Expr::Column(companion(c));
                                let ce = match e {
                                    Expr::Alias(a) => ce.alias_qualified(a.relation.clone(), format!("{}{CT_SUFFIX}", a.name)),
                                    _ => ce,
                                };
                                companions.push(ce);
                            } else {
                                has_ct = false;
                            }
                        }
                        _ => has_ct = false,
                    }
                    if anchored {
                        anchored_out.insert(exprs.len());
                    }
                    Taint::Prot { domain, desc, anchored, has_ct }
                }
                other => other,
            };
            exprs.push(e2);
            taints.push(t);
        }
        for c in companions {
            exprs.push(c);
            taints.push(Taint::Ct);
        }
        let proj = Projection::try_new(exprs, Arc::new(input.plan))?;
        let plan = LogicalPlan::Projection(proj);
        let mut block = Block::default();
        for i in anchored_out {
            block.anchors.insert(key_of(plan.schema(), i));
        }
        Ok(Node { plan, taints, block })
    }

    fn domain_mismatch(da: &str, dsa: &str, db: &str, dsb: &str) -> DepError {
        DepError::dep(
            "DEP_MATCH_DOMAIN_MISMATCH",
            format!("{dsa} (match domain {da}) cannot be compared with {dsb} (match domain {db})."),
            Some("Values from different match domains are different kinds of value."),
        )
    }

    /// Records joins between protected columns, and anchors, from the top-level conjuncts.
    fn scan_conjuncts(&self, pred: &Expr, schema: &DFSchema, taints: &[Taint], block: &mut Block) {
        let prot_col = |e: &Expr| -> Option<(String, String, String)> {
            if let Expr::Column(c) = strip_alias(e) {
                let i = schema.maybe_index_of_column(c)?;
                if let Taint::Prot { domain, desc, .. } = &taints[i] {
                    return Some((key_of(schema, i), desc.clone(), domain.clone()));
                }
            }
            None
        };
        for c in split_conjunction(pred) {
            match c {
                Expr::BinaryExpr(b) if b.op == Operator::Eq => match (prot_col(&b.left), prot_col(&b.right)) {
                    (Some((k, _, _)), None) if is_constant(&b.right) => {
                        block.anchors.insert(k);
                    }
                    (None, Some((k, _, _))) if is_constant(&b.left) => {
                        block.anchors.insert(k);
                    }
                    _ => {}
                },
                Expr::InList(il) if !il.negated && il.list.len() == 1 => {
                    if let Some((k, _, _)) = prot_col(&il.expr) {
                        if is_constant(&il.list[0]) {
                            block.anchors.insert(k);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn drain(&mut self, block: &mut Block) {
        block.edges.extend(std::mem::take(&mut self.pending));
        block.anchors.extend(std::mem::take(&mut self.pending_anchors));
    }

    /// Every join between protected columns must be connected to an anchor.
    fn close(&self, block: &Block) -> Result<()> {
        if block.edges.is_empty() {
            return Ok(());
        }
        let comp = block.components();
        let roots = block.anchored_roots();
        for e in &block.edges {
            let root = comp.get(&e.a).cloned().unwrap_or_else(|| e.a.clone());
            if roots.contains(&root) {
                self.note(format!("Join of PROTECTED columns {} and {} (MATCH DOMAIN {}) is anchored to a single protected value.", e.da, e.db, e.domain));
            }
            if !roots.contains(&root) && !self.links(&e.domain) {
                let (x, y) = if e.db == "a subquery" { (e.da.clone(), e.db.clone()) } else { (e.db.clone(), e.da.clone()) };
                return Err(DepError::dep(
                    "DEP_UNANCHORED_MATCH",
                    if e.db == "a subquery" {
                        format!("Match of PROTECTED column {x} against {y} is not anchored to a single protected value.")
                    } else {
                        format!("Join on PROTECTED columns {x} and {y} is not anchored to a single protected value.")
                    },
                    Some(&format!("Anchor the join with a lookup such as WHERE {x} = $1, or perform it inside a function declared LINKS {}.", e.domain)),
                ));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ expressions

    fn taint_of(&self, c: &Column, schema: &DFSchema, taints: &[Taint]) -> Taint {
        match schema.maybe_index_of_column(c) {
            Some(i) => taints[i].clone(),
            None => Taint::Ord,
        }
    }

    fn rw(&mut self, e: &Expr, schema: &DFSchema, taints: &[Taint], place: Place) -> Result<(Expr, Taint)> {
        match e {
            Expr::Column(c) => {
                let t = self.taint_of(c, schema, taints);
                if let Taint::Disc { desc, .. } = &t {
                    if place != Place::Project && place != Place::Sort {
                        return Err(DepError::disclosed_use(desc, place.desc()));
                    }
                }
                Ok((e.clone(), t))
            }
            Expr::OuterReferenceColumn(_, c) => {
                for s in self.outer.iter().rev() {
                    if let Some(i) = s.schema.maybe_index_of_column(c) {
                        return match &s.taints[i] {
                            Taint::Prot { .. } | Taint::Ct => {
                                Err(DepError::unsupported("Correlated subqueries on PROTECTED columns are not supported. Write the condition as a join."))
                            }
                            Taint::Disc { desc, .. } => Err(DepError::disclosed_use(desc, "a subquery")),
                            Taint::Ord => Ok((e.clone(), Taint::Ord)),
                        };
                    }
                }
                Ok((e.clone(), Taint::Ord))
            }
            Expr::Alias(a) => {
                let (inner, t) = self.rw(&a.expr, schema, taints, place)?;
                Ok((Expr::Alias(Alias { expr: Box::new(inner), relation: a.relation.clone(), name: a.name.clone(), metadata: a.metadata.clone() }), t))
            }
            Expr::Literal(..) | Expr::Placeholder(_) | Expr::ScalarVariable(..) => Ok((e.clone(), Taint::Ord)),
            Expr::BinaryExpr(b) if b.op == Operator::Eq => self.rw_eq(b, schema, taints, place),
            Expr::IsNull(x) | Expr::IsNotNull(x) => {
                let (x2, t) = self.rw(x, schema, taints, place)?;
                if let Taint::Disc { desc, .. } = &t {
                    if place != Place::Project {
                        return Err(DepError::disclosed_use(desc, place.desc()));
                    }
                }
                let out = if matches!(e, Expr::IsNull(_)) { Expr::IsNull(Box::new(x2)) } else { Expr::IsNotNull(Box::new(x2)) };
                Ok((out, Taint::Ord))
            }
            Expr::InList(il) => self.rw_in_list(il, schema, taints, place),
            Expr::InSubquery(isq) => self.rw_in_subquery(isq, schema, taints, place),
            Expr::Exists(ex) => {
                let sub = self.subquery(&ex.subquery, schema, taints)?;
                Ok((Expr::Exists(Exists { subquery: sub.0, negated: ex.negated }), Taint::Ord))
            }
            Expr::ScalarSubquery(sq) => {
                let (sub, t) = self.subquery(sq, schema, taints)?;
                let t = match t {
                    Taint::Prot { domain, desc, anchored, .. } => Taint::Prot { domain, desc, anchored, has_ct: false },
                    other => other,
                };
                Ok((Expr::ScalarSubquery(sub), t))
            }
            Expr::ScalarFunction(f) if f.name() == "reveal" => self.rw_reveal(e, f, schema, taints, place),
            Expr::AggregateFunction(af) if af.func.name() == "count" && !af.params.distinct => {
                // COUNT(x) counts rows; it matches nothing.
                for a in &af.params.args {
                    let (_, t) = self.rw(a, schema, taints, Place::Project)?;
                    if let Taint::Disc { desc, .. } = &t {
                        return Err(DepError::disclosed_use(desc, "an aggregate"));
                    }
                }
                Ok((e.clone(), Taint::Ord))
            }
            Expr::Cast(c) => {
                let (x, t) = self.rw(&c.expr, schema, taints, place)?;
                if let Taint::Prot { desc, .. } = &t {
                    return Err(DepError::dep(
                        "DEP_PROTECTED_OPERATION",
                        format!("Conversion from PROTECTED to plain {} is forbidden ({desc}).", sql_type_name(c.field.data_type())),
                        Some("To release a value, use a disclosing function."),
                    ));
                }
                // In a filter, the cast's input goes through dep_opaque (see udf::Opaque): without it,
                // DataFusion 55 may drop a later ORDER BY on the cast column.
                let x = if matches!(place, Place::Filter | Place::JoinCond) && !is_constant(&x) { OPAQUE.call(vec![x]) } else { x };
                Ok((Expr::Cast(datafusion::logical_expr::Cast { expr: Box::new(x), field: c.field.clone() }), t))
            }
            _ => self.rw_generic(e, schema, taints, place),
        }
    }

    fn rw_generic(&mut self, e: &Expr, schema: &DFSchema, taints: &[Taint], place: Place) -> Result<(Expr, Taint)> {
        let mut child_taints: Vec<Taint> = Vec::new();
        let mut err: Option<DepError> = None;
        let out = e.clone().map_children(|ch| {
            if err.is_some() {
                return Ok(Transformed::no(ch));
            }
            match self.rw(&ch, schema, taints, place) {
                Ok((n, t)) => {
                    child_taints.push(t);
                    Ok(Transformed::yes(n))
                }
                Err(x) => {
                    err = Some(x);
                    Ok(Transformed::no(ch))
                }
            }
        })?;
        if let Some(x) = err {
            return Err(x);
        }
        for t in &child_taints {
            match t {
                Taint::Prot { desc, .. } => return Err(DepError::protected_op(&op_name(e), desc)),
                Taint::Ct => return Err(DepError::unsupported("Hidden columns cannot be used.")),
                _ => {}
            }
        }
        let t = child_taints.into_iter().find(|t| matches!(t, Taint::Disc { .. })).unwrap_or(Taint::Ord);
        if let (Taint::Disc { desc, .. }, true) = (&t, matches!(e, Expr::AggregateFunction(_) | Expr::WindowFunction(_))) {
            return Err(DepError::disclosed_use(desc, "an aggregate"));
        }
        Ok((out.data, t))
    }

    fn rw_eq(&mut self, b: &BinaryExpr, schema: &DFSchema, taints: &[Taint], place: Place) -> Result<(Expr, Taint)> {
        let (l, lt) = self.rw(&b.left, schema, taints, place)?;
        let (r, rt) = self.rw(&b.right, schema, taints, place)?;
        for t in [&lt, &rt] {
            if let Taint::Disc { desc, .. } = t {
                return Err(DepError::disclosed_use(desc, if place == Place::Project { "a comparison" } else { place.desc() }));
            }
        }
        let rebuild = |l: Expr, r: Expr| Expr::BinaryExpr(BinaryExpr::new(Box::new(l), Operator::Eq, Box::new(r)));
        match (&lt, &rt) {
            (Taint::Prot { domain: da, desc: dsa, .. }, Taint::Prot { domain: db, desc: dsb, .. }) => {
                if da != db {
                    return Err(Self::domain_mismatch(da, dsa, db, dsb));
                }
                self.check_match_allowed()?;
                match (strip_alias(&l), strip_alias(&r)) {
                    (Expr::Column(ca), Expr::Column(cb)) => match (schema.maybe_index_of_column(ca), schema.maybe_index_of_column(cb)) {
                        (Some(ia), Some(ib)) => {
                            self.pending.push(Edge { a: key_of(schema, ia), b: key_of(schema, ib), da: dsa.clone(), db: dsb.clone(), domain: da.clone() })
                        }
                        _ => return Err(DepError::unsupported("This comparison of PROTECTED values is not supported.")),
                    },
                    _ => return Err(DepError::unsupported("Compare PROTECTED columns directly.")),
                }
                Ok((rebuild(l, r), Taint::Ord))
            }
            (Taint::Prot { domain, desc, .. }, Taint::Ord) => {
                if !is_constant(&r) {
                    return Err(DepError::protected_op("Comparison with a non-PROTECTED column", desc));
                }
                self.check_match_allowed()?;
                let d = domain.clone();
                self.note(format!("PROTECTED column {desc} is matched by equality through the blind index of MATCH DOMAIN {d}."));
                self.event("match", desc, Some(&d), 1);
                let r2 = self.index_expr(&d, r);
                Ok((rebuild(l, r2), Taint::Ord))
            }
            (Taint::Ord, Taint::Prot { domain, desc, .. }) => {
                if !is_constant(&l) {
                    return Err(DepError::protected_op("Comparison with a non-PROTECTED column", desc));
                }
                self.check_match_allowed()?;
                let d = domain.clone();
                self.note(format!("PROTECTED column {desc} is matched by equality through the blind index of MATCH DOMAIN {d}."));
                self.event("match", desc, Some(&d), 1);
                let l2 = self.index_expr(&d, l);
                Ok((rebuild(l2, r), Taint::Ord))
            }
            (Taint::Ct, _) | (_, Taint::Ct) => Err(DepError::unsupported("Hidden columns cannot be used.")),
            _ => Ok((rebuild(l, r), Taint::Ord)),
        }
    }

    fn rw_in_list(&mut self, il: &InList, schema: &DFSchema, taints: &[Taint], place: Place) -> Result<(Expr, Taint)> {
        let (x, xt) = self.rw(&il.expr, schema, taints, place)?;
        if let Taint::Disc { desc, .. } = &xt {
            return Err(DepError::disclosed_use(desc, place.desc()));
        }
        let mut list = Vec::new();
        match &xt {
            Taint::Prot { domain, desc, .. } => {
                let d = domain.clone();
                for item in &il.list {
                    let (it, t) = self.rw(item, schema, taints, place)?;
                    if t != Taint::Ord || !is_constant(&it) {
                        return Err(DepError::protected_op("IN with a non-constant list", desc));
                    }
                    list.push(self.index_expr(&d, it));
                }
                self.check_match_allowed()?;
                self.note(format!("PROTECTED column {desc} is matched against a list of values through the blind index of MATCH DOMAIN {d}."));
                self.event("match", desc, Some(&d), il.list.len() as u32);
            }
            _ => {
                for item in &il.list {
                    let (it, t) = self.rw(item, schema, taints, place)?;
                    if let Taint::Prot { desc, .. } = &t {
                        return Err(DepError::protected_op("IN", desc));
                    }
                    list.push(it);
                }
            }
        }
        Ok((Expr::InList(InList { expr: Box::new(x), list, negated: il.negated }), Taint::Ord))
    }

    fn rw_in_subquery(&mut self, isq: &InSubquery, schema: &DFSchema, taints: &[Taint], place: Place) -> Result<(Expr, Taint)> {
        let (x, xt) = self.rw(&isq.expr, schema, taints, place)?;
        if let Taint::Disc { desc, .. } = &xt {
            return Err(DepError::disclosed_use(desc, place.desc()));
        }
        let (sub, st) = self.subquery(&isq.subquery, schema, taints)?;
        match (&xt, &st) {
            (Taint::Prot { domain: da, desc: dsa, .. }, Taint::Prot { domain: db, desc: dsb, anchored, .. }) => {
                if da != db {
                    return Err(Self::domain_mismatch(da, dsa, db, dsb));
                }
                self.check_match_allowed()?;
                let Some(i) = (match strip_alias(&x) {
                    Expr::Column(c) => schema.maybe_index_of_column(c),
                    _ => None,
                }) else {
                    return Err(DepError::unsupported("Match a PROTECTED column directly against a subquery."));
                };
                self.seq += 1;
                let syn = format!("#subquery{}", self.seq);
                if *anchored {
                    self.pending_anchors.insert(syn.clone());
                }
                self.pending.push(Edge { a: key_of(schema, i), b: syn, da: dsa.clone(), db: "a subquery".into(), domain: da.clone() });
            }
            (Taint::Prot { desc, .. }, _) | (_, Taint::Prot { desc, .. }) => {
                return Err(DepError::protected_op("Comparison with a non-PROTECTED column", desc))
            }
            (_, Taint::Disc { desc, .. }) => return Err(DepError::disclosed_use(desc, place.desc())),
            _ => {}
        }
        Ok((Expr::InSubquery(InSubquery { expr: Box::new(x), subquery: sub, negated: isq.negated }), Taint::Ord))
    }

    /// Analyzes a subquery in its own block. Returns it without hidden columns,
    /// and the kind of its first output column.
    fn subquery(&mut self, sq: &Subquery, schema: &DFSchema, taints: &[Taint]) -> Result<(Subquery, Taint)> {
        self.outer.push(Scope { schema: schema.clone(), taints: taints.to_vec() });
        let saved = (std::mem::take(&mut self.pending), std::mem::take(&mut self.pending_anchors));
        let res = self.analyze(&sq.subquery);
        self.pending = saved.0;
        self.pending_anchors = saved.1;
        self.outer.pop();
        let node = res?;
        self.close(&node.block)?;
        let mut plan = node.plan;
        let visible: Vec<usize> = (0..node.taints.len()).filter(|&i| node.taints[i] != Taint::Ct).collect();
        if visible.len() != node.taints.len() {
            let s = plan.schema().clone();
            let exprs = visible.iter().map(|&i| {
                let (q, f) = s.qualified_field(i);
                Expr::Column(Column::new(q.cloned(), f.name()))
            });
            plan = LogicalPlan::Projection(Projection::try_new(exprs.collect(), Arc::new(plan))?);
        }
        let first = visible.first().map(|&i| node.taints[i].clone()).unwrap_or(Taint::Ord);
        Ok((Subquery { subquery: Arc::new(plan), outer_ref_columns: sq.outer_ref_columns.clone(), spans: sq.spans.clone() }, first))
    }

    fn rw_reveal(
        &mut self,
        _e: &Expr,
        f: &datafusion::logical_expr::expr::ScalarFunction,
        schema: &DFSchema,
        taints: &[Taint],
        place: Place,
    ) -> Result<(Expr, Taint)> {
        if f.args.len() != 1 {
            return Err(DepError::sql("42883", "REVEAL takes one argument"));
        }
        let arg = strip_alias(&f.args[0]);
        let (col, t) = match arg {
            Expr::Column(c) => (c.clone(), self.taint_of(c, schema, taints)),
            _ => return Err(DepError::sql("42804", "REVEAL takes a PROTECTED column")),
        };
        let Taint::Prot { domain, desc, has_ct, .. } = t else {
            return Err(DepError::sql("42804", "REVEAL takes a PROTECTED column"));
        };
        let allowed = self.region.as_ref().map(|r| r.discloses.contains(&domain)).unwrap_or(false);
        if !allowed {
            return Err(DepError::dep(
                "DEP_REVEAL_OUTSIDE_DISCLOSURE",
                format!("REVEAL() of {desc} (match domain {domain}) appears outside a disclosing function."),
                Some(&format!("Move the disclosure into a function declared DISCLOSES {domain}.")),
            ));
        }
        if place != Place::Project {
            return Err(DepError::disclosed_use(&format!("REVEAL({desc})"), place.desc()));
        }
        let ct = companion(&col);
        if !has_ct || schema.maybe_index_of_column(&ct).is_none() {
            return Err(DepError::unsupported(format!("REVEAL of {desc} must read the column directly, not through an aggregate or a scalar subquery.")));
        }
        let out = self.decrypt_udf.call(vec![lit(domain.clone()), Expr::Column(ct)]);
        Ok((out, Taint::Disc { domain, desc }))
    }

    // ------------------------------------------------------------------ outputs

    /// The plan a client receives: protected columns become NULL and hidden columns are dropped.
    pub fn finalize(node: Node) -> Result<LogicalPlan> {
        let schema = node.plan.schema().clone();
        let mut exprs = Vec::new();
        for (i, t) in node.taints.iter().enumerate() {
            let (q, f) = schema.qualified_field(i);
            match t {
                Taint::Ct => continue,
                Taint::Prot { .. } => exprs.push(lit(ScalarValue::Utf8(None)).alias_qualified(q.cloned(), f.name())),
                _ => exprs.push(Expr::Column(Column::new(q.cloned(), f.name()))),
            }
        }
        Ok(LogicalPlan::Projection(Projection::try_new(exprs, Arc::new(node.plan))?))
    }

    /// Column names a client sees, in order.
    pub fn visible(node: &Node) -> Vec<(Option<TableReference>, String, Taint)> {
        let schema = node.plan.schema();
        node.taints
            .iter()
            .enumerate()
            .filter(|(_, t)| **t != Taint::Ct)
            .map(|(i, t)| {
                let (q, f) = schema.qualified_field(i);
                (q.cloned(), f.name().clone(), t.clone())
            })
            .collect()
    }

    pub fn close_top(&self, node: &Node) -> Result<()> {
        self.close(&node.block)
    }
}

pub fn is_prot(t: &Taint) -> bool {
    t.is_prot()
}
