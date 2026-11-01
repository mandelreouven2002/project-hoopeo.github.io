//! psql-style text output.

use datafusion::arrow::array::Array;
use datafusion::arrow::util::display::array_value_to_string;

use crate::server::Output;

pub fn render(out: &Output) -> String {
    match out {
        Output::Command(tag) => tag.clone(),
        Output::Rows { schema, batches } => {
            let headers: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
            let mut rows: Vec<Vec<String>> = Vec::new();
            for b in batches {
                for r in 0..b.num_rows() {
                    rows.push(
                        (0..b.num_columns())
                            .map(|c| {
                                let col = b.column(c);
                                if col.is_null(r) {
                                    "NULL".to_string()
                                } else {
                                    array_value_to_string(col, r).unwrap_or_default()
                                }
                            })
                            .collect(),
                    );
                }
            }
            let mut w: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
            for row in &rows {
                for (i, v) in row.iter().enumerate() {
                    w[i] = w[i].max(v.chars().count());
                }
            }
            let line = |cells: &[String], center: bool| -> String {
                cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let pad = w[i] - c.chars().count();
                        if center {
                            let l = pad / 2;
                            format!(" {}{}{} ", " ".repeat(l), c, " ".repeat(pad - l))
                        } else {
                            format!(" {}{} ", c, " ".repeat(pad))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("|")
            };
            let mut s = String::new();
            s.push_str(line(&headers, true).trim_end());
            s.push('\n');
            s.push_str(&w.iter().map(|x| "-".repeat(x + 2)).collect::<Vec<_>>().join("+"));
            s.push('\n');
            for row in &rows {
                s.push_str(line(row, false).trim_end());
                s.push('\n');
            }
            s.push_str(&format!("({} row{})", rows.len(), if rows.len() == 1 { "" } else { "s" }));
            s
        }
    }
}

/// psql-style text for a client reply.
pub fn render_reply(r: &crate::client::Reply) -> String {
    use crate::client::Reply;
    match r {
        Reply::Command(t) => t.clone(),
        Reply::Rows { columns, rows } => table(columns, rows),
    }
}

pub fn table(headers: &[String], rows: &[Vec<Option<String>>]) -> String {
    let cells: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|v| v.clone().unwrap_or_else(|| "NULL".into())).collect()).collect();
    let mut w: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &cells {
        for (i, v) in row.iter().enumerate() {
            if i < w.len() {
                w[i] = w[i].max(v.lines().map(|l| l.chars().count()).max().unwrap_or(0));
            }
        }
    }
    let line = |cells: &[String], center: bool| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let c = c.replace('\n', " ");
                let pad = w[i].saturating_sub(c.chars().count());
                if center {
                    let l = pad / 2;
                    format!(" {}{}{} ", " ".repeat(l), c, " ".repeat(pad - l))
                } else {
                    format!(" {}{} ", c, " ".repeat(pad))
                }
            })
            .collect::<Vec<_>>()
            .join("|")
    };
    let mut s = String::new();
    s.push_str(line(headers, true).trim_end());
    s.push('\n');
    s.push_str(&w.iter().map(|x| "-".repeat(x + 2)).collect::<Vec<_>>().join("+"));
    s.push('\n');
    for row in &cells {
        s.push_str(line(row, false).trim_end());
        s.push('\n');
    }
    s.push_str(&format!("({} row{})", rows.len(), if rows.len() == 1 { "" } else { "s" }));
    s
}
