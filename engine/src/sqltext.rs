//! The parts of DEP SQL that standard SQL parsers do not know: statement
//! splitting that respects `$$` bodies, the `PROTECTED MATCH DOMAIN` column
//! option, `CREATE FUNCTION ... DISCLOSES ... LINKS`, and `CREATE ENDPOINT`.
//! Everything else is parsed by sqlparser-rs, the parser DataFusion uses.

use datafusion::sql::sqlparser::dialect::PostgreSqlDialect;
use datafusion::sql::sqlparser::tokenizer::{Token, Tokenizer, Whitespace};

use crate::error::{DepError, Result};

pub fn tokens(sql: &str) -> Result<Vec<Token>> {
    Tokenizer::new(&PostgreSqlDialect {}, sql).tokenize().map_err(|e| DepError::sql("42601", e.to_string()))
}

fn is_ws(t: &Token) -> bool {
    matches!(t, Token::Whitespace(_))
}

fn is_comment_or_ws(t: &Token) -> bool {
    matches!(t, Token::Whitespace(_) | Token::EOF)
}

/// Splits a script into statements at semicolons outside strings and `$$` bodies.
pub fn split(sql: &str) -> Result<Vec<String>> {
    let toks = tokens(sql)?;
    let mut out = Vec::new();
    let mut cur: Vec<Token> = Vec::new();
    let flush = |cur: &mut Vec<Token>, out: &mut Vec<String>| {
        if cur.iter().any(|t| !is_comment_or_ws(t)) {
            let s: String = cur.iter().map(|t| t.to_string()).collect();
            out.push(s.trim().to_string());
        }
        cur.clear();
    };
    for t in toks {
        if t == Token::SemiColon {
            flush(&mut cur, &mut out);
        } else {
            cur.push(t);
        }
    }
    flush(&mut cur, &mut out);
    Ok(out)
}

/// The first few words of a statement, upper-cased, ignoring comments.
pub fn leading_words(sql: &str, n: usize) -> Vec<String> {
    let Ok(toks) = tokens(sql) else { return vec![] };
    toks.iter()
        .filter(|t| !is_ws(t))
        .take(n)
        .map(|t| match t {
            Token::Word(w) if w.quote_style.is_none() => w.value.to_uppercase(),
            other => other.to_string(),
        })
        .collect()
}

/// Rejects references to the hidden ciphertext columns and to the internal functions.
pub fn check_reserved(sql: &str) -> Result<()> {
    let toks = tokens(sql)?;
    let sig: Vec<&Token> = toks.iter().filter(|t| !is_ws(t)).collect();
    for (i, t) in sig.iter().enumerate() {
        if let Token::Word(w) = t {
            let v = w.value.to_lowercase();
            if v.ends_with(crate::catalog::CT_SUFFIX) {
                return Err(DepError::sql("42703", format!("column \"{}\" does not exist", w.value)));
            }
            if v.starts_with("dep_") && matches!(sig.get(i + 1), Some(Token::LParen)) {
                return Err(DepError::sql("42883", format!("function {}() does not exist", w.value)));
            }
        }
    }
    Ok(())
}

pub struct ProtectedOption {
    pub column: String,
    pub domain: Option<String>,
}

/// Removes `PROTECTED [MATCH DOMAIN name]` from a CREATE TABLE statement and
/// reports which columns carried it. The rest is ordinary SQL.
pub fn strip_protected(sql: &str) -> Result<(String, Vec<ProtectedOption>)> {
    let toks = tokens(sql)?;
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    let mut found = Vec::new();
    let mut depth = 0i32;
    let mut expect_item = false;
    let mut current: Option<String> = None;
    let mut i = 0;
    let next_sig = |from: usize| -> Option<usize> { (from..toks.len()).find(|&k| !is_ws(&toks[k])) };
    while i < toks.len() {
        let t = &toks[i];
        match t {
            Token::LParen => {
                depth += 1;
                if depth == 1 {
                    expect_item = true;
                }
            }
            Token::RParen => depth -= 1,
            Token::Comma if depth == 1 => expect_item = true,
            Token::Word(w) if depth == 1 && expect_item => {
                expect_item = false;
                let u = w.value.to_uppercase();
                current = if w.quote_style.is_none() && ["PRIMARY", "FOREIGN", "CONSTRAINT", "UNIQUE", "CHECK"].contains(&u.as_str()) {
                    None
                } else {
                    Some(if w.quote_style.is_some() { w.value.clone() } else { w.value.to_lowercase() })
                };
            }
            Token::Word(w) if depth == 1 && w.quote_style.is_none() && w.value.eq_ignore_ascii_case("PROTECTED") => {
                let col = current.clone().ok_or_else(|| DepError::sql("42601", "PROTECTED applies to a column"))?;
                let mut domain = None;
                let mut j = i + 1;
                if let Some(a) = next_sig(j) {
                    if matches!(&toks[a], Token::Word(w) if w.value.eq_ignore_ascii_case("MATCH")) {
                        let b = next_sig(a + 1).ok_or_else(|| DepError::sql("42601", "syntax error after MATCH"))?;
                        if !matches!(&toks[b], Token::Word(w) if w.value.eq_ignore_ascii_case("DOMAIN")) {
                            return Err(DepError::sql("42601", "expected DOMAIN after MATCH"));
                        }
                        let c = next_sig(b + 1).ok_or_else(|| DepError::sql("42601", "expected a match domain name"))?;
                        match &toks[c] {
                            Token::Word(w) => domain = Some(w.value.to_lowercase()),
                            _ => return Err(DepError::sql("42601", "expected a match domain name")),
                        }
                        j = c + 1;
                    }
                }
                found.push(ProtectedOption { column: col, domain });
                i = j;
                continue;
            }
            _ => {}
        }
        out.push(t.clone());
        i += 1;
    }
    Ok((out.iter().map(|t| t.to_string()).collect(), found))
}

#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub or_replace: bool,
    pub name: String,
    pub params: Vec<(String, String)>,
    pub returns: Vec<(String, String)>,
    pub language: Option<String>,
    pub discloses: Vec<String>,
    pub links: Vec<String>,
    pub body: String,
}

struct Cursor {
    t: Vec<Token>,
    i: usize,
}

impl Cursor {
    fn new(sql: &str) -> Result<Self> {
        Ok(Cursor { t: tokens(sql)?.into_iter().filter(|t| !is_ws(t) && *t != Token::EOF).collect(), i: 0 })
    }
    fn peek(&self) -> Option<&Token> {
        self.t.get(self.i)
    }
    fn next(&mut self) -> Result<Token> {
        let t = self.t.get(self.i).cloned().ok_or_else(|| DepError::sql("42601", "syntax error at end of input"))?;
        self.i += 1;
        Ok(t)
    }
    fn kw(&mut self, k: &str) -> bool {
        if matches!(self.peek(), Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(k)) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect_kw(&mut self, k: &str) -> Result<()> {
        if self.kw(k) {
            Ok(())
        } else {
            Err(self.err())
        }
    }
    fn expect(&mut self, t: Token) -> Result<()> {
        if self.peek() == Some(&t) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err())
        }
    }
    fn err(&self) -> DepError {
        match self.peek() {
            Some(t) => DepError::sql("42601", format!("syntax error at or near \"{t}\"")),
            None => DepError::sql("42601", "syntax error at end of input"),
        }
    }
    fn ident(&mut self) -> Result<String> {
        match self.next()? {
            Token::Word(w) => Ok(if w.quote_style.is_some() { w.value } else { w.value.to_lowercase() }),
            _ => {
                self.i -= 1;
                Err(self.err())
            }
        }
    }
    /// A type name: everything up to a comma or closing parenthesis at this level.
    fn type_text(&mut self) -> Result<String> {
        let mut depth = 0;
        let mut parts: Vec<String> = Vec::new();
        while let Some(t) = self.peek() {
            match t {
                Token::LParen => depth += 1,
                Token::RParen if depth == 0 => break,
                Token::RParen => depth -= 1,
                Token::Comma if depth == 0 => break,
                _ => {}
            }
            parts.push(self.next()?.to_string());
        }
        if parts.is_empty() {
            return Err(self.err());
        }
        Ok(parts.join(" ").replace(" ( ", "(").replace(" )", ")"))
    }
    fn name_type_list(&mut self) -> Result<Vec<(String, String)>> {
        self.expect(Token::LParen)?;
        let mut v = Vec::new();
        if self.peek() == Some(&Token::RParen) {
            self.i += 1;
            return Ok(v);
        }
        loop {
            let n = self.ident()?;
            let t = self.type_text()?;
            v.push((n, t));
            match self.next()? {
                Token::Comma => continue,
                Token::RParen => break,
                _ => {
                    self.i -= 1;
                    return Err(self.err());
                }
            }
        }
        Ok(v)
    }
    fn ident_list(&mut self) -> Result<Vec<String>> {
        let mut v = vec![self.ident()?];
        while self.peek() == Some(&Token::Comma) {
            self.i += 1;
            v.push(self.ident()?);
        }
        Ok(v)
    }
}

pub fn parse_create_function(sql: &str) -> Result<FunctionDef> {
    let mut c = Cursor::new(sql)?;
    c.expect_kw("CREATE")?;
    let or_replace = if c.kw("OR") {
        c.expect_kw("REPLACE")?;
        true
    } else {
        false
    };
    c.expect_kw("FUNCTION")?;
    let name = c.ident()?;
    let params = c.name_type_list()?;
    c.expect_kw("RETURNS")?;
    if !c.kw("TABLE") {
        return Err(DepError::unsupported("Declared functions must use RETURNS TABLE (...)."));
    }
    let returns = c.name_type_list()?;
    let mut def = FunctionDef { or_replace, name, params, returns, language: None, discloses: vec![], links: vec![], body: String::new() };
    while c.peek().is_some() {
        if c.kw("LANGUAGE") {
            def.language = Some(c.ident()?);
        } else if c.kw("DISCLOSES") {
            def.discloses.extend(c.ident_list()?);
        } else if c.kw("LINKS") {
            def.links.extend(c.ident_list()?);
        } else if c.kw("AS") {
            def.body = match c.next()? {
                Token::DollarQuotedString(d) => d.value,
                Token::SingleQuotedString(s) => s,
                _ => {
                    c.i -= 1;
                    return Err(c.err());
                }
            };
        } else if c.kw("IMMUTABLE") || c.kw("STABLE") || c.kw("VOLATILE") || c.kw("STRICT") {
        } else {
            return Err(c.err());
        }
    }
    if def.body.trim().is_empty() {
        return Err(DepError::sql("42P13", "function body is missing (AS $$ ... $$)"));
    }
    Ok(def)
}

/// Replaces references to the parameters in a function body with typed placeholders.
pub fn bind_params(body: &str, params: &[(String, String)]) -> Result<String> {
    let toks = tokens(body)?;
    let sig: Vec<usize> = (0..toks.len()).filter(|&k| !is_ws(&toks[k])).collect();
    let mut out = String::new();
    for (k, t) in toks.iter().enumerate() {
        if let Token::Word(w) = t {
            if w.quote_style.is_none() {
                let pos = sig.iter().position(|&s| s == k);
                let prev = pos.and_then(|p| p.checked_sub(1)).map(|p| &toks[sig[p]]);
                let next = pos.and_then(|p| sig.get(p + 1)).map(|&p| &toks[p]);
                let qualified = matches!(prev, Some(Token::Period)) || matches!(next, Some(Token::Period) | Some(Token::LParen));
                if !qualified {
                    if let Some(ix) = params.iter().position(|(n, _)| n.eq_ignore_ascii_case(&w.value)) {
                        out.push_str(&format!("CAST(${} AS {})", ix + 1, params[ix].1));
                        continue;
                    }
                }
            }
        }
        out.push_str(&t.to_string());
    }
    Ok(out)
}

/// `CREATE ENDPOINT name MODE OPERATIONAL | MASKED_READ`
pub fn parse_create_endpoint(sql: &str) -> Result<(String, String)> {
    let mut c = Cursor::new(sql)?;
    c.expect_kw("CREATE")?;
    c.expect_kw("ENDPOINT")?;
    let name = c.ident()?;
    c.expect_kw("MODE")?;
    let mode = c.ident()?.to_uppercase();
    if c.peek().is_some() {
        return Err(c.err());
    }
    Ok((name, mode))
}

/// `COMMENT ON FUNCTION name(...) IS '...'`
pub fn parse_comment_on_function(sql: &str) -> Result<(String, Option<String>)> {
    let mut c = Cursor::new(sql)?;
    c.expect_kw("COMMENT")?;
    c.expect_kw("ON")?;
    c.expect_kw("FUNCTION")?;
    let name = c.ident()?;
    if c.peek() == Some(&Token::LParen) {
        let mut depth = 0;
        loop {
            match c.next()? {
                Token::LParen => depth += 1,
                Token::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    c.expect_kw("IS")?;
    let text = match c.next()? {
        Token::SingleQuotedString(s) => Some(s),
        Token::Word(w) if w.value.eq_ignore_ascii_case("NULL") => None,
        _ => return Err(c.err()),
    };
    Ok((name, text))
}

/// `DROP FUNCTION [IF EXISTS] name[(...)]` and `DROP TABLE [IF EXISTS] name`
pub fn parse_drop(sql: &str) -> Result<(String, bool, String)> {
    let mut c = Cursor::new(sql)?;
    c.expect_kw("DROP")?;
    let kind = c.ident()?.to_uppercase();
    let if_exists = if c.kw("IF") {
        c.expect_kw("EXISTS")?;
        true
    } else {
        false
    };
    let name = c.ident()?;
    Ok((kind, if_exists, name))
}

pub fn is_whitespace_only(sql: &str) -> bool {
    tokens(sql)
        .map(|t| {
            t.iter().all(|t| matches!(t, Token::Whitespace(Whitespace::Space | Whitespace::Newline | Whitespace::Tab) | Token::Whitespace(_) | Token::EOF))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strip() {
        let (s, f) =
            strip_protected("CREATE TABLE t (id BIGINT PRIMARY KEY, nid TEXT PROTECTED MATCH DOMAIN person_id UNIQUE, x TEXT PROTECTED REFERENCES p (nid))")
                .unwrap();
        assert!(!s.to_uppercase().contains("PROTECTED"));
        assert!(s.contains("UNIQUE"));
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].column, "nid");
        assert_eq!(f[0].domain.as_deref(), Some("person_id"));
        assert_eq!(f[1].domain, None);
    }
    #[test]
    fn function() {
        let d = parse_create_function("CREATE OR REPLACE FUNCTION f(id TEXT, d DATE) RETURNS TABLE (a TEXT, b DOUBLE PRECISION) LANGUAGE sql DISCLOSES person_id LINKS person_id AS $$ SELECT 1 $$").unwrap();
        assert!(d.or_replace);
        assert_eq!(d.params.len(), 2);
        assert_eq!(d.returns[1].1, "DOUBLE PRECISION");
        assert_eq!(d.discloses, vec!["person_id"]);
        let b = bind_params("SELECT p.id, id FROM t AS p WHERE x = id", &d.params).unwrap();
        assert_eq!(b, "SELECT p.id, CAST($1 AS TEXT) FROM t AS p WHERE x = CAST($1 AS TEXT)");
    }
    #[test]
    fn splitting() {
        let v = split("SELECT 1; CREATE FUNCTION f() RETURNS TABLE (a TEXT) AS $$ SELECT 'a;b' $$; -- c\n").unwrap();
        assert_eq!(v.len(), 2);
    }
}

// ---------------------------------------------------------------------- roles and privileges

#[derive(Debug, Clone, Default)]
pub struct RoleOptions {
    pub login: Option<bool>,
    pub superuser: Option<bool>,
    pub password: Option<Option<String>>,
    /// `MATCH LIMIT n PER unit | NONE | DEFAULT`
    pub match_limit: Option<crate::audit::RoleLimit>,
}

/// `CREATE ROLE name [WITH] [LOGIN | NOLOGIN] [SUPERUSER | NOSUPERUSER] [PASSWORD 'x' | PASSWORD NULL]`,
/// `CREATE USER ...` (LOGIN by default), and `ALTER ROLE | USER name [WITH] ...`.
pub fn parse_role(sql: &str) -> Result<(String, String, RoleOptions)> {
    let mut c = Cursor::new(sql)?;
    let verb = c.ident()?.to_uppercase();
    let kind = c.ident()?.to_uppercase();
    if !["ROLE", "USER"].contains(&kind.as_str()) {
        return Err(DepError::sql("42601", format!("syntax error at or near \"{kind}\"")));
    }
    let name = c.ident()?;
    let mut o = RoleOptions::default();
    if kind == "USER" && verb == "CREATE" {
        o.login = Some(true);
    }
    c.kw("WITH");
    while c.peek().is_some() {
        if c.kw("LOGIN") {
            o.login = Some(true);
        } else if c.kw("NOLOGIN") {
            o.login = Some(false);
        } else if c.kw("SUPERUSER") {
            o.superuser = Some(true);
        } else if c.kw("NOSUPERUSER") {
            o.superuser = Some(false);
        } else if c.kw("MATCH") {
            if !c.kw("LIMIT") {
                return Err(c.err());
            }
            o.match_limit = Some(if c.kw("NONE") {
                crate::audit::RoleLimit::Unlimited
            } else if c.kw("DEFAULT") {
                crate::audit::RoleLimit::Default
            } else {
                let count: u32 = match c.next()? {
                    Token::Number(n, _) => n.parse().map_err(|_| DepError::sql("22023", format!("invalid match limit \"{n}\"")))?,
                    _ => {
                        c.i -= 1;
                        return Err(c.err());
                    }
                };
                if !c.kw("PER") {
                    return Err(c.err());
                }
                let unit = c.ident()?;
                let per_secs = crate::audit::unit_secs(&unit).ok_or_else(|| DepError::sql("22023", format!("unknown period \"{unit}\"")).with_hint("Use SECOND, MINUTE, HOUR or DAY."))?;
                crate::audit::RoleLimit::Limit(crate::audit::MatchLimit { count, per_secs })
            });
        } else if c.kw("PASSWORD") {
            o.password = Some(match c.next()? {
                Token::SingleQuotedString(s) => Some(s),
                Token::Word(w) if w.value.eq_ignore_ascii_case("NULL") => None,
                _ => {
                    c.i -= 1;
                    return Err(c.err());
                }
            });
        } else {
            return Err(c.err());
        }
    }
    Ok((verb, name, o))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Object {
    Table(Vec<String>),
    Function(String),
    MatchDomain(String),
    Endpoint(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Privilege {
    Select(Option<Vec<String>>),
    Insert,
    Update(Option<Vec<String>>),
    Delete,
    All,
    Execute,
    Disclose,
    Link,
    Connect,
}

#[derive(Debug, Clone)]
pub struct GrantStmt {
    pub grant: bool,
    pub privileges: Vec<Privilege>,
    pub object: Object,
    pub roles: Vec<String>,
}

/// GRANT | REVOKE privileges ON [TABLE] t | FUNCTION f(...) | MATCH DOMAIN d | ENDPOINT e  TO | FROM roles
pub fn parse_grant(sql: &str) -> Result<GrantStmt> {
    let mut c = Cursor::new(sql)?;
    let grant = if c.kw("GRANT") {
        true
    } else {
        c.expect_kw("REVOKE")?;
        false
    };
    let mut privileges = Vec::new();
    loop {
        let w = c.ident()?.to_uppercase();
        let cols = |c: &mut Cursor| -> Result<Option<Vec<String>>> {
            if c.peek() == Some(&Token::LParen) {
                c.i += 1;
                let v = c.ident_list()?;
                c.expect(Token::RParen)?;
                Ok(Some(v))
            } else {
                Ok(None)
            }
        };
        privileges.push(match w.as_str() {
            "SELECT" => Privilege::Select(cols(&mut c)?),
            "UPDATE" => Privilege::Update(cols(&mut c)?),
            "INSERT" => Privilege::Insert,
            "DELETE" => Privilege::Delete,
            "ALL" => {
                c.kw("PRIVILEGES");
                Privilege::All
            }
            "EXECUTE" => Privilege::Execute,
            "DISCLOSE" => Privilege::Disclose,
            "LINK" => Privilege::Link,
            "CONNECT" => Privilege::Connect,
            other => return Err(DepError::sql("42601", format!("unrecognized privilege type \"{}\"", other.to_lowercase()))),
        });
        if c.peek() == Some(&Token::Comma) {
            c.i += 1;
        } else {
            break;
        }
    }
    c.expect_kw("ON")?;
    let object = if c.kw("FUNCTION") {
        let name = c.ident()?;
        if c.peek() == Some(&Token::LParen) {
            let mut depth = 0;
            loop {
                match c.next()? {
                    Token::LParen => depth += 1,
                    Token::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        Object::Function(name)
    } else if c.kw("MATCH") {
        c.expect_kw("DOMAIN")?;
        Object::MatchDomain(c.ident()?)
    } else if c.kw("ENDPOINT") {
        Object::Endpoint(c.ident()?)
    } else {
        c.kw("TABLE");
        Object::Table(c.ident_list()?)
    };
    if grant {
        c.expect_kw("TO")?;
    } else {
        c.expect_kw("FROM")?;
    }
    let roles = c.ident_list()?;
    if c.peek().is_some() {
        return Err(c.err());
    }
    Ok(GrantStmt { grant, privileges, object, roles })
}

/// The highest `$n` placeholder in a statement (0 when there is none).
pub fn placeholder_count(sql: &str) -> Result<usize> {
    Ok(tokens(sql)?
        .iter()
        .filter_map(|t| match t {
            Token::Placeholder(p) => p.strip_prefix('$').and_then(|n| n.parse::<usize>().ok()),
            _ => None,
        })
        .max()
        .unwrap_or(0))
}

/// Replaces `$n` placeholders with SQL literals (already quoted), outside strings and comments.
/// This is how the extended query protocol and the HTTP API bind parameters.
pub fn substitute_params(sql: &str, literals: &[String]) -> Result<String> {
    let mut out = String::with_capacity(sql.len());
    for t in tokens(sql)? {
        match &t {
            Token::Placeholder(p) if p.starts_with('$') => {
                let n: usize = p[1..].parse().map_err(|_| DepError::sql("42P02", format!("there is no parameter {p}")))?;
                let lit = literals.get(n.wrapping_sub(1)).ok_or_else(|| DepError::sql("42P02", format!("there is no parameter {p}")))?;
                out.push_str(lit);
            }
            Token::EOF => {}
            other => out.push_str(&other.to_string()),
        }
    }
    Ok(out)
}

/// Removes the values of bound parameters from a message that goes back to a client (an
/// error message can quote a value, for example a value that could not be converted). Each
/// value becomes the placeholder it was bound to: quoted forms always, bare forms when they
/// are at least 4 characters long (shorter ones would match ordinary words of the message).
pub fn scrub_values(text: &str, values: &[String]) -> String {
    let mut order: Vec<(usize, &String)> = values.iter().enumerate().filter(|(_, v)| !v.is_empty()).collect();
    order.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    let mut out = text.to_string();
    for (i, v) in order {
        let ph = format!("${}", i + 1);
        for quoted in [format!("'{}'", v.replace('\'', "''")), format!("'{v}'"), format!("\"{v}\"")] {
            out = out.replace(&quoted, &ph);
        }
        if v.chars().count() >= 4 {
            out = out.replace(v.as_str(), &ph);
        }
    }
    out
}

/// A string as a SQL literal.
pub fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
