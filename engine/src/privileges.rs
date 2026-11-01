//! Which table columns, tables, and declared functions a plan uses, so that
//! ordinary SQL privileges can be checked. In DEP, privilege on a protected
//! column is the privilege to match on it: a role that may not name a column
//! may not look values up through it either.

use std::collections::BTreeSet;

use datafusion::catalog::default_table_source::source_as_provider;
use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
use datafusion::common::DFSchema;
use datafusion::logical_expr::{Expr, LogicalPlan};

use crate::catalog::{Catalog, CT_SUFFIX};
use crate::functions::DepFunctionTable;

#[derive(Debug, Default, Clone)]
pub struct Uses {
    /// (table, column) pairs named anywhere in the query.
    pub columns: BTreeSet<(String, String)>,
    /// Tables read at all.
    pub tables: BTreeSet<String>,
    /// Declared functions called.
    pub functions: BTreeSet<String>,
}

type Origins = Vec<Option<(String, String)>>;

struct Walker<'a> {
    catalog: &'a Catalog,
    uses: Uses,
    outer: Vec<(DFSchema, Origins)>,
}

pub fn collect(plan: &LogicalPlan, catalog: &Catalog) -> Uses {
    let mut w = Walker { catalog, uses: Uses::default(), outer: vec![] };
    w.walk(plan);
    w.uses
}

impl Walker<'_> {
    fn resolve(&mut self, e: &Expr, schema: &DFSchema, origins: &Origins) {
        let _ = e.apply(|x| {
            match x {
                Expr::Column(c) => {
                    if let Some(i) = schema.maybe_index_of_column(c) {
                        if let Some(o) = origins.get(i).cloned().flatten() {
                            self.uses.columns.insert(o);
                        }
                    }
                }
                Expr::OuterReferenceColumn(_, c) => {
                    for (s, o) in self.outer.iter().rev() {
                        if let Some(i) = s.maybe_index_of_column(c) {
                            if let Some(o) = o.get(i).cloned().flatten() {
                                self.uses.columns.insert(o);
                            }
                            break;
                        }
                    }
                }
                Expr::ScalarSubquery(sq) => self.sub(&sq.subquery, schema, origins),
                Expr::Exists(ex) => self.sub(&ex.subquery.subquery, schema, origins),
                Expr::InSubquery(is) => self.sub(&is.subquery.subquery, schema, origins),
                _ => {}
            }
            Ok(TreeNodeRecursion::Continue)
        });
    }

    fn sub(&mut self, plan: &LogicalPlan, schema: &DFSchema, origins: &Origins) {
        self.outer.push((schema.clone(), origins.clone()));
        self.walk(plan);
        self.outer.pop();
    }

    fn origin_of(e: &Expr, schema: &DFSchema, origins: &Origins) -> Option<(String, String)> {
        let mut e = e;
        while let Expr::Alias(a) = e {
            e = &a.expr;
        }
        match e {
            Expr::Column(c) => schema.maybe_index_of_column(c).and_then(|i| origins.get(i).cloned().flatten()),
            _ => None,
        }
    }

    fn walk(&mut self, plan: &LogicalPlan) -> Origins {
        match plan {
            LogicalPlan::TableScan(ts) => {
                if let Ok(p) = source_as_provider(&ts.source) {
                    if let Some(f) = (p.as_ref() as &dyn std::any::Any).downcast_ref::<DepFunctionTable>() {
                        self.uses.functions.insert(f.meta.name.clone());
                        return vec![None; plan.schema().fields().len()];
                    }
                }
                let t = ts.table_name.table().to_string();
                if self.catalog.tables.contains_key(&t) {
                    self.uses.tables.insert(t.clone());
                    return plan
                        .schema()
                        .fields()
                        .iter()
                        .map(|f| Some((t.clone(), f.name().strip_suffix(CT_SUFFIX).unwrap_or(f.name()).to_string())))
                        .collect();
                }
                vec![None; plan.schema().fields().len()]
            }
            LogicalPlan::Join(j) => {
                let lo = self.walk(&j.left);
                let ro = self.walk(&j.right);
                for (l, r) in &j.on {
                    self.resolve(l, j.left.schema(), &lo);
                    self.resolve(r, j.right.schema(), &ro);
                }
                let n = j.schema.fields().len();
                let mut out: Origins = if n == lo.len() + ro.len() {
                    lo.into_iter().chain(ro).collect()
                } else if j.schema.qualified_field(0) == j.left.schema().qualified_field(0) {
                    lo
                } else {
                    ro
                };
                out.resize(n, None);
                if let Some(f) = &j.filter {
                    self.resolve(f, &j.schema, &out);
                }
                out
            }
            _ => {
                let inputs = plan.inputs();
                let ins: Vec<Origins> = inputs.iter().map(|i| self.walk(i)).collect();
                let (schema, origins) = match inputs.first() {
                    Some(i) => (i.schema().as_ref().clone(), ins[0].clone()),
                    None => (DFSchema::empty(), vec![]),
                };
                for e in plan.expressions() {
                    self.resolve(&e, &schema, &origins);
                }
                let n = plan.schema().fields().len();
                match plan {
                    LogicalPlan::Projection(p) => p.expr.iter().map(|e| Self::origin_of(e, &schema, &origins)).collect(),
                    LogicalPlan::Aggregate(a) => {
                        let mut v: Origins = a.group_expr.iter().map(|e| Self::origin_of(e, &schema, &origins)).collect();
                        v.resize(n, None);
                        v
                    }
                    LogicalPlan::Filter(_)
                    | LogicalPlan::Sort(_)
                    | LogicalPlan::Limit(_)
                    | LogicalPlan::Distinct(_)
                    | LogicalPlan::SubqueryAlias(_)
                    | LogicalPlan::Window(_) => {
                        let mut v = origins;
                        v.resize(n, None);
                        v
                    }
                    _ => vec![None; n],
                }
            }
        }
    }
}
