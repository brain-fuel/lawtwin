//! LawSpec types for the simulated peers: the type of a topic's values, read
//! from a LawSpec source, values of it generated from a seed, and values
//! checked against it.
//!
//! A peer reads the data declarations it needs: `type` declarations
//! (products and sums with named fields, whose fields may be refined),
//! `refinement` declarations without parameters, and `definition`s whose
//! body is one expression, which a refinement may call. A refinement's
//! predicate may use literals, the value and its fields, arithmetic,
//! comparisons, `&&`, `||`, `!`, `if`, `matches regex`, `prelude.length`,
//! `prelude.isEmpty`, `prelude.isPresent` and those definitions. Anything
//! else in the source is skipped. Values are generated within the bounds a
//! predicate's comparisons give, and kept only when the whole predicate
//! holds. ref:DEC-peer-value-types

use crate::peer::Rng;
use base64::Engine as _;
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

/// A type.
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    /// A fixed-width integer: bits and whether it is signed.
    Int(u8, bool),
    /// An integer of any size.
    Integer,
    /// A float.
    Float,
    /// `Bool`.
    Bool,
    /// `Text`.
    Text,
    /// `Bytes`.
    Bytes,
    /// `Unit`.
    Unit,
    /// `Maybe a`.
    Maybe(Box<Ty>),
    /// `List a`.
    List(Box<Ty>),
    /// `Either a b`.
    Either(Box<Ty>, Box<Ty>),
    /// A declared data type or refinement.
    Named(String),
    /// `(v :: T where p)`.
    Refined(String, Box<Ty>, Box<Expr>),
}

/// An expression of a predicate.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// An integer literal.
    Int(i128),
    /// A float literal.
    Float(f64),
    /// A text literal.
    Text(String),
    /// `true` or `false`.
    Bool(bool),
    /// A name: a variable, a field path (`v.amount`), or a function.
    Name(String),
    /// A function applied to arguments.
    App(String, Vec<Expr>),
    /// A binary operator.
    Bin(String, Box<Expr>, Box<Expr>),
    /// `!e` or `-e`.
    Un(String, Box<Expr>),
    /// `if c then a else b`.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `e matches regex "..."`.
    Matches(Box<Expr>, String),
}

/// A constructor with its fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Ctor {
    /// Its name.
    pub name: String,
    /// Its fields, in order.
    pub fields: Vec<(String, Ty)>,
}

/// A definition a predicate may call.
#[derive(Clone, Debug, PartialEq)]
pub struct Def {
    /// Its parameters.
    pub params: Vec<String>,
    /// Its body.
    pub body: Expr,
}

/// The declarations read from LawSpec sources.
#[derive(Clone, Debug, Default)]
pub struct Types {
    /// Data types, by name.
    pub data: BTreeMap<String, Vec<Ctor>>,
    /// Refinements, by name.
    pub refinements: BTreeMap<String, Ty>,
    /// Definitions, by name.
    pub defs: BTreeMap<String, Def>,
}

/// A value of a type.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    /// An integer.
    Int(i128),
    /// A float.
    Float(f64),
    /// A Boolean.
    Bool(bool),
    /// Text.
    Text(String),
    /// Bytes.
    Bytes(Vec<u8>),
    /// Unit.
    Unit,
    /// A list.
    List(Vec<V>),
    /// `Nothing` or `Just v`.
    Maybe(Option<Box<V>>),
    /// `Left v` (false) or `Right v` (true).
    Either(bool, Box<V>),
    /// A constructor and its fields.
    Data(String, Vec<(String, V)>),
}

// ---------------------------------------------------------------------------
// Reading LawSpec
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Id(String),
    Num(String),
    Str(String),
    Sym(String),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '{' && chars.get(i + 1) == Some(&'-') {
            while i + 1 < chars.len() && !(chars[i] == '-' && chars[i + 1] == '}') {
                i += 1;
            }
            i += 2;
        } else if c == '"' {
            let mut s = String::new();
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    s.push(match chars[i] {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if c == '`' {
            let mut s = String::new();
            i += 1;
            while i < chars.len() && chars[i] != '`' {
                s.push(chars[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Id(s));
        } else if c.is_ascii_digit() {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_' || (chars[i] == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))) {
                if chars[i] != '_' {
                    s.push(chars[i]);
                }
                i += 1;
            }
            out.push(Tok::Num(s));
        } else if c.is_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '\'' || (chars[i] == '.' && chars.get(i + 1).is_some_and(|d| d.is_alphabetic()))) {
                s.push(chars[i]);
                i += 1;
            }
            out.push(Tok::Id(s));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            if ["::", "->", "==", "!=", "<=", ">=", "&&", "||"].contains(&two.as_str()) {
                out.push(Tok::Sym(two));
                i += 2;
            } else {
                out.push(Tok::Sym(c.to_string()));
                i += 1;
            }
        }
    }
    out
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

type PResult<T> = Result<T, String>;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn peek_n(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.at + n)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn is_id(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Id(x)) if x == w)
    }

    fn is_sym(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Sym(x)) if x == w)
    }

    fn expect_id(&mut self, w: &str) -> PResult<()> {
        if self.is_id(w) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected {w}, found {:?}", self.peek()))
        }
    }

    fn expect_sym(&mut self, w: &str) -> PResult<()> {
        if self.is_sym(w) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected {w}, found {:?}", self.peek()))
        }
    }

    fn ident(&mut self) -> PResult<String> {
        match self.next() {
            Some(Tok::Id(s)) => Ok(s),
            t => Err(format!("expected a name, found {t:?}")),
        }
    }

    /// Skips to the `end` that closes a block already opened.
    fn skip_block(&mut self) {
        let mut depth = 1;
        while let Some(t) = self.next() {
            match t {
                Tok::Id(w) if w == "is" || w == "match" => depth += 1,
                Tok::Id(w) if w == "end" => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn type_atom(&mut self) -> PResult<Ty> {
        if self.is_sym("(") {
            self.at += 1;
            // `(v :: T where p)` or a parenthesised type.
            if matches!(self.peek(), Some(Tok::Id(v)) if v.starts_with(|c: char| c.is_lowercase())) && matches!(self.peek_n(1), Some(Tok::Sym(s)) if s == "::") {
                let var = self.ident()?;
                self.expect_sym("::")?;
                let base = self.type_expr()?;
                self.expect_id("where")?;
                let pred = self.expr()?;
                self.expect_sym(")")?;
                return Ok(Ty::Refined(var, Box::new(base), Box::new(pred)));
            }
            let t = self.type_expr()?;
            self.expect_sym(")")?;
            return Ok(t);
        }
        let name = self.ident()?;
        Ok(match name.as_str() {
            "Int8" => Ty::Int(8, true),
            "Int16" => Ty::Int(16, true),
            "Int32" => Ty::Int(32, true),
            "Int64" | "Int" => Ty::Int(64, true),
            "UInt8" => Ty::Int(8, false),
            "UInt16" => Ty::Int(16, false),
            "UInt32" => Ty::Int(32, false),
            "UInt64" => Ty::Int(64, false),
            "Integer" | "BigInt" | "Natural" => Ty::Integer,
            "Float" | "Float32" | "Float64" | "Double" => Ty::Float,
            "Bool" => Ty::Bool,
            "Text" | "CodePointText" | "Utf16Text" => Ty::Text,
            "Bytes" => Ty::Bytes,
            "Unit" => Ty::Unit,
            other => Ty::Named(other.rsplit('.').next().unwrap_or(other).to_string()),
        })
    }

    fn type_expr(&mut self) -> PResult<Ty> {
        let head = self.type_atom()?;
        let arity = match &head {
            Ty::Named(n) if ["Maybe", "List", "Nullable", "Optional", "Set"].contains(&n.as_str()) => 1,
            Ty::Named(n) if n == "Either" => 2,
            _ => 0,
        };
        let mut args = vec![];
        for _ in 0..arity {
            args.push(self.type_atom()?);
        }
        Ok(match &head {
            Ty::Named(n) if n == "Maybe" || n == "Nullable" || n == "Optional" => Ty::Maybe(Box::new(args.remove(0))),
            Ty::Named(n) if n == "List" || n == "Set" => Ty::List(Box::new(args.remove(0))),
            Ty::Named(n) if n == "Either" => {
                let b = args.remove(1);
                Ty::Either(Box::new(args.remove(0)), Box::new(b))
            }
            _ => head,
        })
    }

    fn data(&mut self) -> PResult<(String, Vec<Ctor>)> {
        let name = self.ident()?;
        if !self.is_id("is") {
            return Err(format!("the type {name} takes parameters, which a peer does not read"));
        }
        self.expect_id("is")?;
        let mut ctors = vec![];
        loop {
            if self.is_id("end") {
                self.at += 1;
                break;
            }
            if self.is_sym("|") {
                self.at += 1;
            }
            let cname = self.ident()?;
            let mut fields = vec![];
            while matches!(self.peek(), Some(Tok::Id(f)) if f.starts_with(|c: char| c.is_lowercase()) && f != "end")
                && matches!(self.peek_n(1), Some(Tok::Sym(s)) if s == "::")
            {
                let f = self.ident()?;
                self.expect_sym("::")?;
                fields.push((f, self.type_expr()?));
            }
            ctors.push(Ctor { name: cname, fields });
        }
        Ok((name, ctors))
    }

    fn refinement(&mut self) -> PResult<(String, Ty)> {
        let name = self.ident()?;
        if !self.is_id("is") {
            return Err(format!("the refinement {name} takes parameters, which a peer does not read"));
        }
        self.expect_id("is")?;
        let t = self.type_atom()?;
        self.expect_id("end")?;
        Ok((name, t))
    }

    fn definition(&mut self) -> PResult<(String, Def)> {
        let name = self.ident()?;
        let mut params = vec![];
        while self.is_sym("(") {
            self.at += 1;
            params.push(self.ident()?);
            self.expect_sym("::")?;
            self.type_expr()?;
            self.expect_sym(")")?;
        }
        self.expect_sym("::")?;
        self.type_expr()?;
        if self.is_id("fails") {
            return Err(format!("{name} can fail"));
        }
        self.expect_id("is")?;
        let body = self.expr()?;
        self.expect_id("end")?;
        Ok((name, Def { params, body }))
    }

    fn expr(&mut self) -> PResult<Expr> {
        if self.is_id("if") {
            self.at += 1;
            let c = self.expr()?;
            self.expect_id("then")?;
            let a = self.expr()?;
            self.expect_id("else")?;
            let b = self.expr()?;
            return Ok(Expr::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        self.or()
    }

    fn or(&mut self) -> PResult<Expr> {
        let mut l = self.and()?;
        while self.is_sym("||") {
            self.at += 1;
            let r = self.and()?;
            l = Expr::Bin("||".into(), Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn and(&mut self) -> PResult<Expr> {
        let mut l = self.cmp()?;
        while self.is_sym("&&") {
            self.at += 1;
            let r = self.cmp()?;
            l = Expr::Bin("&&".into(), Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn cmp(&mut self) -> PResult<Expr> {
        let l = self.add()?;
        if self.is_id("matches") {
            self.at += 1;
            self.expect_id("regex")?;
            return match self.next() {
                Some(Tok::Str(re)) => Ok(Expr::Matches(Box::new(l), re)),
                t => Err(format!("expected a regex, found {t:?}")),
            };
        }
        for op in ["==", "!=", "<=", ">=", "<", ">", "="] {
            if self.is_sym(op) {
                self.at += 1;
                let r = self.add()?;
                let op = if op == "=" { "==" } else { op };
                return Ok(Expr::Bin(op.into(), Box::new(l), Box::new(r)));
            }
        }
        Ok(l)
    }

    fn add(&mut self) -> PResult<Expr> {
        let mut l = self.mul()?;
        while self.is_sym("+") || self.is_sym("-") {
            let op = match self.next() {
                Some(Tok::Sym(s)) => s,
                _ => unreachable!(),
            };
            let r = self.mul()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn mul(&mut self) -> PResult<Expr> {
        let mut l = self.unary()?;
        while self.is_sym("*") || self.is_sym("/") {
            let op = match self.next() {
                Some(Tok::Sym(s)) => s,
                _ => unreachable!(),
            };
            let r = self.unary()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn unary(&mut self) -> PResult<Expr> {
        if self.is_sym("!") || self.is_sym("-") {
            let op = match self.next() {
                Some(Tok::Sym(s)) => s,
                _ => unreachable!(),
            };
            let e = self.unary()?;
            return Ok(Expr::Un(op, Box::new(e)));
        }
        self.app()
    }

    fn starts_atom(&self) -> bool {
        match self.peek() {
            Some(Tok::Num(_)) | Some(Tok::Str(_)) => true,
            Some(Tok::Sym(s)) => s == "(",
            Some(Tok::Id(w)) => !["then", "else", "end", "matches", "is", "where", "with", "in"].contains(&w.as_str()),
            None => false,
        }
    }

    fn app(&mut self) -> PResult<Expr> {
        let head = self.atom()?;
        if let Expr::Name(f) = &head {
            let mut args = vec![];
            while self.starts_atom() {
                args.push(self.atom()?);
            }
            if !args.is_empty() {
                return Ok(Expr::App(f.clone(), args));
            }
        }
        Ok(head)
    }

    fn atom(&mut self) -> PResult<Expr> {
        match self.next() {
            Some(Tok::Num(n)) => {
                if n.contains('.') {
                    n.parse().map(Expr::Float).map_err(|e| e.to_string())
                } else {
                    n.parse().map(Expr::Int).map_err(|e| e.to_string())
                }
            }
            Some(Tok::Str(s)) => Ok(Expr::Text(s)),
            Some(Tok::Id(w)) if w == "true" => Ok(Expr::Bool(true)),
            Some(Tok::Id(w)) if w == "false" => Ok(Expr::Bool(false)),
            Some(Tok::Id(w)) => Ok(Expr::Name(w)),
            Some(Tok::Sym(s)) if s == "(" => {
                let e = self.expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            t => Err(format!("unexpected {t:?} in an expression")),
        }
    }
}

impl Types {
    /// Reads the declarations of LawSpec sources. A declaration a peer
    /// cannot read is skipped; a type that needs it says so when used.
    pub fn read(sources: &[String]) -> Self {
        let mut types = Types::default();
        for src in sources {
            let mut p = Parser { toks: tokenize(src), at: 0 };
            let mut depth = 0i32;
            while let Some(t) = p.peek().cloned() {
                match t {
                    Tok::Id(w) if depth == 0 && w == "type" => {
                        p.at += 1;
                        let start = p.at;
                        match p.data() {
                            Ok((n, c)) => {
                                types.data.insert(n, c);
                            }
                            Err(_) => {
                                p.at = start;
                                while !p.is_id("is") && p.peek().is_some() {
                                    p.at += 1;
                                }
                                p.at += 1;
                                p.skip_block();
                            }
                        }
                    }
                    Tok::Id(w) if depth == 0 && w == "refinement" => {
                        p.at += 1;
                        let start = p.at;
                        match p.refinement() {
                            Ok((n, t)) => {
                                types.refinements.insert(n, t);
                            }
                            Err(_) => {
                                p.at = start;
                                while !p.is_id("is") && p.peek().is_some() {
                                    p.at += 1;
                                }
                                p.at += 1;
                                p.skip_block();
                            }
                        }
                    }
                    Tok::Id(w) if depth == 0 && w == "definition" => {
                        p.at += 1;
                        let start = p.at;
                        match p.definition() {
                            Ok((n, d)) => {
                                types.defs.insert(n, d);
                            }
                            Err(_) => {
                                p.at = start;
                                while !p.is_id("is") && p.peek().is_some() {
                                    p.at += 1;
                                }
                                p.at += 1;
                                p.skip_block();
                            }
                        }
                    }
                    Tok::Id(w) if w == "is" || w == "match" => {
                        depth += 1;
                        p.at += 1;
                    }
                    Tok::Id(w) if w == "end" => {
                        depth = (depth - 1).max(0);
                        p.at += 1;
                    }
                    _ => p.at += 1,
                }
            }
        }
        types
    }

    /// Reads the declarations of every `.lawspec` file at a path: a file,
    /// or a directory searched below.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let mut sources = vec![];
        let mut stack = vec![path.to_path_buf()];
        while let Some(p) = stack.pop() {
            if p.is_dir() {
                for e in std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))? {
                    stack.push(e.map_err(|e| e.to_string())?.path());
                }
            } else if p.extension().is_some_and(|x| x == "lawspec") {
                sources.push(std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?);
            }
        }
        if sources.is_empty() {
            return Err(format!("no LawSpec source at {}", path.display()));
        }
        Ok(Self::read(&sources))
    }

    /// A type named in a declaration, or written as an expression such as
    /// `List Order`.
    pub fn parse_type(&self, text: &str) -> Result<Ty, String> {
        let mut p = Parser { toks: tokenize(text), at: 0 };
        let t = p.type_expr()?;
        if p.peek().is_some() {
            return Err(format!("{text} is not one type"));
        }
        self.check_known(&t)?;
        Ok(t)
    }

    fn check_known(&self, t: &Ty) -> Result<(), String> {
        match t {
            Ty::Named(n) if !self.data.contains_key(n) && !self.refinements.contains_key(n) => {
                Err(format!("no type or refinement {n} that a peer can read"))
            }
            Ty::Maybe(a) | Ty::List(a) => self.check_known(a),
            Ty::Either(a, b) => self.check_known(a).and(self.check_known(b)),
            Ty::Refined(_, b, _) => self.check_known(b),
            _ => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Evaluating predicates
// ---------------------------------------------------------------------------

type Env = Vec<(String, V)>;

fn field(v: &V, name: &str) -> Result<V, String> {
    match v {
        V::Data(_, fields) => fields.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).ok_or_else(|| format!("no field {name}")),
        _ => Err(format!("no field {name} on {v:?}")),
    }
}

fn truth(v: V) -> Result<bool, String> {
    match v {
        V::Bool(b) => Ok(b),
        other => Err(format!("{other:?} is not a Boolean")),
    }
}

impl Types {
    fn lookup(&self, env: &Env, name: &str) -> Result<V, String> {
        let mut parts = name.split('.');
        let first = parts.next().unwrap_or("");
        if let Some((_, v)) = env.iter().rev().find(|(n, _)| n == first) {
            let mut v = v.clone();
            for p in parts {
                v = field(&v, p)?;
            }
            return Ok(v);
        }
        Err(format!("{name} is not known to the peer"))
    }

    /// Evaluates an expression.
    pub fn eval(&self, env: &Env, e: &Expr) -> Result<V, String> {
        Ok(match e {
            Expr::Int(n) => V::Int(*n),
            Expr::Float(f) => V::Float(*f),
            Expr::Text(s) => V::Text(s.clone()),
            Expr::Bool(b) => V::Bool(*b),
            Expr::Name(n) => match self.lookup(env, n) {
                Ok(v) => v,
                Err(err) => {
                    let short = n.rsplit('.').next().unwrap_or(n);
                    match self.defs.get(short) {
                        Some(d) if d.params.is_empty() => self.eval(&vec![], &d.body)?,
                        _ => match short {
                            "unitValue" => V::Unit,
                            "Nothing" => V::Maybe(None),
                            _ => return Err(err),
                        },
                    }
                }
            },
            Expr::App(f, args) => {
                let vals: Vec<V> = args.iter().map(|a| self.eval(env, a)).collect::<Result<_, _>>()?;
                let short = f.rsplit('.').next().unwrap_or(f);
                match (short, vals.as_slice()) {
                    ("length", [V::Text(s)]) => V::Int(s.chars().count() as i128),
                    ("length", [V::Bytes(b)]) => V::Int(b.len() as i128),
                    ("length", [V::List(l)]) => V::Int(l.len() as i128),
                    ("isEmpty", [V::Text(s)]) => V::Bool(s.is_empty()),
                    ("isEmpty", [V::Bytes(b)]) => V::Bool(b.is_empty()),
                    ("isEmpty", [V::List(l)]) => V::Bool(l.is_empty()),
                    ("isPresent", [V::Maybe(m)]) => V::Bool(m.is_some()),
                    ("presentValue", [V::Maybe(Some(v))]) => (**v).clone(),
                    ("Just", [v]) => V::Maybe(Some(Box::new(v.clone()))),
                    _ => match self.defs.get(short) {
                        Some(d) if d.params.len() == vals.len() => {
                            let env: Env = d.params.iter().cloned().zip(vals).collect();
                            self.eval(&env, &d.body)?
                        }
                        _ => return Err(format!("the peer cannot evaluate {f}")),
                    },
                }
            }
            Expr::Un(op, a) => match (op.as_str(), self.eval(env, a)?) {
                ("!", V::Bool(b)) => V::Bool(!b),
                ("-", V::Int(n)) => V::Int(-n),
                ("-", V::Float(f)) => V::Float(-f),
                (op, v) => return Err(format!("{op} on {v:?}")),
            },
            Expr::If(c, a, b) => {
                if truth(self.eval(env, c)?)? {
                    self.eval(env, a)?
                } else {
                    self.eval(env, b)?
                }
            }
            Expr::Matches(a, re) => match self.eval(env, a)? {
                V::Text(s) => {
                    let r = regex::Regex::new(&format!("^(?:{re})$")).map_err(|e| e.to_string())?;
                    V::Bool(r.is_match(&s))
                }
                v => return Err(format!("matches on {v:?}")),
            },
            Expr::Bin(op, a, b) => {
                if op == "&&" {
                    return Ok(V::Bool(truth(self.eval(env, a)?)? && truth(self.eval(env, b)?)?));
                }
                if op == "||" {
                    return Ok(V::Bool(truth(self.eval(env, a)?)? || truth(self.eval(env, b)?)?));
                }
                let (x, y) = (self.eval(env, a)?, self.eval(env, b)?);
                match (op.as_str(), &x, &y) {
                    ("==", _, _) => V::Bool(x == y),
                    ("!=", _, _) => V::Bool(x != y),
                    ("+", V::Int(p), V::Int(q)) => V::Int(p + q),
                    ("-", V::Int(p), V::Int(q)) => V::Int(p - q),
                    ("*", V::Int(p), V::Int(q)) => V::Int(p * q),
                    ("/", V::Int(p), V::Int(q)) if *q != 0 => V::Int(p / q),
                    ("+", V::Float(p), V::Float(q)) => V::Float(p + q),
                    ("-", V::Float(p), V::Float(q)) => V::Float(p - q),
                    ("*", V::Float(p), V::Float(q)) => V::Float(p * q),
                    ("+", V::Text(p), V::Text(q)) => V::Text(format!("{p}{q}")),
                    (op, V::Int(p), V::Int(q)) => V::Bool(match op {
                        "<" => p < q,
                        "<=" => p <= q,
                        ">" => p > q,
                        ">=" => p >= q,
                        _ => return Err(format!("{op} on integers")),
                    }),
                    (op, V::Float(p), V::Float(q)) => V::Bool(match op {
                        "<" => p < q,
                        "<=" => p <= q,
                        ">" => p > q,
                        ">=" => p >= q,
                        _ => return Err(format!("{op} on floats")),
                    }),
                    (op, V::Text(p), V::Text(q)) => V::Bool(match op {
                        "<" => p < q,
                        "<=" => p <= q,
                        ">" => p > q,
                        ">=" => p >= q,
                        _ => return Err(format!("{op} on text")),
                    }),
                    (op, x, y) => return Err(format!("{op} on {x:?} and {y:?}")),
                }
            }
        })
    }

    fn holds(&self, var: &str, v: &V, pred: &Expr) -> Result<bool, String> {
        truth(self.eval(&vec![(var.to_string(), v.clone())], pred)?)
    }
}

// ---------------------------------------------------------------------------
// Generating values
// ---------------------------------------------------------------------------

/// What a predicate's comparisons bound.
#[derive(Clone, Debug, Default)]
struct Bounds {
    lo: Option<i128>,
    hi: Option<i128>,
    len_lo: Option<i128>,
    len_hi: Option<i128>,
    regex: Option<String>,
}

fn literal(e: &Expr) -> Option<i128> {
    match e {
        Expr::Int(n) => Some(*n),
        Expr::Un(op, a) if op == "-" => literal(a).map(|n| -n),
        _ => None,
    }
}

fn bounds(types: &Types, var: &str, e: &Expr, b: &mut Bounds) {
    match e {
        Expr::Bin(op, l, r) if op == "&&" => {
            bounds(types, var, l, b);
            bounds(types, var, r, b);
        }
        // A definition called on the value: its body, over its parameter.
        Expr::App(f, args) if args.len() == 1 && matches!(&args[0], Expr::Name(n) if n == var) => {
            if let Some(d) = types.defs.get(f.rsplit('.').next().unwrap_or(f)).filter(|d| d.params.len() == 1) {
                bounds(types, &d.params[0], &d.body, b);
            }
        }
        Expr::Matches(a, re) if matches!(&**a, Expr::Name(n) if n == var) => b.regex = Some(re.clone()),
        Expr::Bin(op, l, r) => {
            let is_var = |e: &Expr| matches!(e, Expr::Name(n) if n == var);
            let is_len = |e: &Expr| matches!(e, Expr::App(f, args) if f.ends_with("length") && args.len() == 1 && is_var(&args[0]));
            let (subject, n, op) = if let Some(n) = literal(r) {
                (&**l, n, op.as_str())
            } else if let Some(n) = literal(l) {
                (
                    &**r,
                    n,
                    match op.as_str() {
                        "<" => ">",
                        "<=" => ">=",
                        ">" => "<",
                        ">=" => "<=",
                        other => other,
                    },
                )
            } else {
                return;
            };
            let (lo, hi) = if is_var(subject) {
                (&mut b.lo, &mut b.hi)
            } else if is_len(subject) {
                (&mut b.len_lo, &mut b.len_hi)
            } else {
                return;
            };
            match op {
                ">" => *lo = Some(lo.map_or(n + 1, |x| x.max(n + 1))),
                ">=" => *lo = Some(lo.map_or(n, |x| x.max(n))),
                "<" => *hi = Some(hi.map_or(n - 1, |x| x.min(n - 1))),
                "<=" => *hi = Some(hi.map_or(n, |x| x.min(n))),
                "==" => {
                    *lo = Some(n);
                    *hi = Some(n);
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn range_of(bits: u8, signed: bool) -> (i128, i128) {
    if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    }
}

const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

fn from_regex(rng: &mut Rng, hir: &regex_syntax::hir::Hir, out: &mut String) {
    use regex_syntax::hir::{Class, HirKind};
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => {}
        HirKind::Literal(l) => out.push_str(&String::from_utf8_lossy(&l.0)),
        HirKind::Class(Class::Unicode(c)) => {
            let ranges: Vec<_> = c.ranges().iter().filter(|r| (r.start() as u32) < 0x10000).collect();
            if let Some(r) = ranges.get(rng.below(ranges.len().max(1) as u64) as usize) {
                let (a, b) = (r.start() as u32, (r.end() as u32).min(0xFFFF));
                let ascii = a < 128 && b >= 32;
                let (a, b) = if ascii { (a.max(32), b.min(126)) } else { (a, b) };
                let c = char::from_u32(a + rng.below((b - a + 1) as u64) as u32).unwrap_or('a');
                out.push(c);
            }
        }
        HirKind::Class(Class::Bytes(c)) => {
            if let Some(r) = c.ranges().first() {
                let (a, b) = (r.start(), r.end().min(126).max(r.start()));
                out.push((a + rng.below((b - a + 1) as u64) as u8) as char);
            }
        }
        HirKind::Repetition(rep) => {
            let max = rep.max.unwrap_or(rep.min + 6).min(rep.min + 6);
            let n = rep.min + rng.below((max - rep.min + 1) as u64) as u32;
            for _ in 0..n {
                from_regex(rng, &rep.sub, out);
            }
        }
        HirKind::Capture(c) => from_regex(rng, &c.sub, out),
        HirKind::Concat(parts) => {
            for p in parts {
                from_regex(rng, p, out);
            }
        }
        HirKind::Alternation(alts) => {
            let i = rng.below(alts.len() as u64) as usize;
            from_regex(rng, &alts[i], out);
        }
    }
}

impl Types {
    /// A value of a type, from the generator.
    pub fn generate(&self, t: &Ty, rng: &mut Rng) -> Result<V, String> {
        self.gen_within(t, rng, &Bounds::default(), 0)
    }

    fn gen_within(&self, t: &Ty, rng: &mut Rng, b: &Bounds, depth: usize) -> Result<V, String> {
        if depth > 12 {
            return Err("a type nests too deeply to generate".into());
        }
        Ok(match t {
            Ty::Int(bits, signed) => {
                let (min, max) = range_of(*bits, *signed);
                let lo = b.lo.unwrap_or(if *signed { -1000 } else { 0 }).max(min);
                let hi = b.hi.unwrap_or(lo.max(0) + 1000).min(max);
                if lo > hi {
                    return Err(format!("no Int{bits} between {lo} and {hi}"));
                }
                V::Int(lo + rng.below((hi - lo + 1).min(u64::MAX as i128) as u64) as i128)
            }
            Ty::Integer => {
                let lo = b.lo.unwrap_or(-1000);
                let hi = b.hi.unwrap_or(lo.max(0) + 1000);
                if lo > hi {
                    return Err(format!("no Integer between {lo} and {hi}"));
                }
                V::Int(lo + rng.below((hi - lo + 1).min(u64::MAX as i128) as u64) as i128)
            }
            Ty::Float => V::Float(rng.below(1_000_000) as f64 / 100.0),
            Ty::Bool => V::Bool(rng.below(2) == 1),
            Ty::Unit => V::Unit,
            Ty::Text => {
                if let Some(re) = &b.regex {
                    let hir = regex_syntax::parse(re).map_err(|e| e.to_string())?;
                    let mut s = String::new();
                    from_regex(rng, &hir, &mut s);
                    V::Text(s)
                } else {
                    let lo = b.len_lo.unwrap_or(1).max(0) as u64;
                    let hi = b.len_hi.unwrap_or(lo as i128 + 11).max(lo as i128) as u64;
                    let n = lo + rng.below(hi - lo + 1);
                    V::Text((0..n).map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize] as char).collect())
                }
            }
            Ty::Bytes => {
                let lo = b.len_lo.unwrap_or(0).max(0) as u64;
                let hi = b.len_hi.unwrap_or(lo as i128 + 16).max(lo as i128) as u64;
                let n = lo + rng.below(hi - lo + 1);
                V::Bytes((0..n).map(|_| rng.below(256) as u8).collect())
            }
            Ty::Maybe(a) => {
                if rng.below(4) == 0 {
                    V::Maybe(None)
                } else {
                    V::Maybe(Some(Box::new(self.gen_within(a, rng, &Bounds::default(), depth + 1)?)))
                }
            }
            Ty::List(a) => {
                let lo = b.len_lo.unwrap_or(0).max(0) as u64;
                let hi = b.len_hi.unwrap_or(lo as i128 + 3).max(lo as i128) as u64;
                let n = lo + rng.below(hi - lo + 1);
                V::List((0..n).map(|_| self.gen_within(a, rng, &Bounds::default(), depth + 1)).collect::<Result<_, _>>()?)
            }
            Ty::Either(l, r) => {
                if rng.below(2) == 0 {
                    V::Either(false, Box::new(self.gen_within(l, rng, &Bounds::default(), depth + 1)?))
                } else {
                    V::Either(true, Box::new(self.gen_within(r, rng, &Bounds::default(), depth + 1)?))
                }
            }
            Ty::Refined(var, base, pred) => {
                let mut within = Bounds::default();
                bounds(self, var, pred, &mut within);
                let mut last = String::new();
                for _ in 0..2000 {
                    let v = self.gen_within(base, rng, &within, depth + 1)?;
                    match self.holds(var, &v, pred) {
                        Ok(true) => return Ok(v),
                        Ok(false) => {}
                        Err(e) => last = e,
                    }
                }
                return Err(format!("no value satisfying the refinement was found{}", if last.is_empty() { String::new() } else { format!(": {last}") }));
            }
            Ty::Named(n) => {
                if let Some(r) = self.refinements.get(n) {
                    return self.gen_within(r, rng, b, depth + 1);
                }
                let ctors = self.data.get(n).ok_or_else(|| format!("no type {n}"))?;
                let c = &ctors[rng.below(ctors.len() as u64) as usize];
                let mut fields = vec![];
                for (f, ft) in &c.fields {
                    fields.push((f.clone(), self.gen_within(ft, rng, &Bounds::default(), depth + 1)?));
                }
                V::Data(c.name.clone(), fields)
            }
        })
    }

    /// Whether a value is of a type: each way it is not, with its path.
    pub fn check(&self, t: &Ty, v: &V, path: &str) -> Vec<String> {
        let mut out = vec![];
        self.check_into(t, v, path, &mut out);
        out
    }

    fn check_into(&self, t: &Ty, v: &V, path: &str, out: &mut Vec<String>) {
        match (t, v) {
            (Ty::Int(bits, signed), V::Int(n)) => {
                let (lo, hi) = range_of(*bits, *signed);
                if *n < lo || *n > hi {
                    out.push(format!("{path}: {n} is not an {}Int{bits}", if *signed { "" } else { "U" }));
                }
            }
            (Ty::Integer, V::Int(_)) | (Ty::Float, V::Float(_)) | (Ty::Float, V::Int(_)) | (Ty::Bool, V::Bool(_)) => {}
            (Ty::Text, V::Text(_)) | (Ty::Bytes, V::Bytes(_)) | (Ty::Unit, V::Unit) => {}
            (Ty::Maybe(_), V::Maybe(None)) => {}
            (Ty::Maybe(a), V::Maybe(Some(x))) => self.check_into(a, x, path, out),
            (Ty::List(a), V::List(xs)) => {
                for (i, x) in xs.iter().enumerate() {
                    self.check_into(a, x, &format!("{path}[{i}]"), out);
                }
            }
            (Ty::Either(l, _), V::Either(false, x)) => self.check_into(l, x, path, out),
            (Ty::Either(_, r), V::Either(true, x)) => self.check_into(r, x, path, out),
            (Ty::Refined(var, base, pred), v) => {
                let before = out.len();
                self.check_into(base, v, path, out);
                if out.len() == before {
                    match self.holds(var, v, pred) {
                        Ok(true) => {}
                        Ok(false) => out.push(format!("{path}: {} does not satisfy its refinement", show(v))),
                        Err(e) => out.push(format!("{path}: the refinement cannot be checked: {e}")),
                    }
                }
            }
            (Ty::Named(n), v) => {
                if let Some(r) = self.refinements.get(n) {
                    return self.check_into(r, v, path, out);
                }
                let Some(ctors) = self.data.get(n) else {
                    out.push(format!("{path}: no type {n}"));
                    return;
                };
                match v {
                    V::Data(c, fields) => match ctors.iter().find(|x| &x.name == c) {
                        None => out.push(format!("{path}: {c} is not a constructor of {n}")),
                        Some(ctor) => {
                            for (f, ft) in &ctor.fields {
                                match fields.iter().find(|(x, _)| x == f) {
                                    None => out.push(format!("{path}.{f}: missing")),
                                    Some((_, fv)) => self.check_into(ft, fv, &format!("{path}.{f}"), out),
                                }
                            }
                        }
                    },
                    other => out.push(format!("{path}: {} is not a {n}", show(other))),
                }
            }
            (t, v) => out.push(format!("{path}: {} is not of type {}", show(v), type_name(t))),
        }
    }
}

fn show(v: &V) -> String {
    let s = format!("{:?}", to_json_untyped(v));
    if s.len() > 80 { format!("{}...", &s[..80]) } else { s }
}

fn type_name(t: &Ty) -> String {
    match t {
        Ty::Int(b, true) => format!("Int{b}"),
        Ty::Int(b, false) => format!("UInt{b}"),
        Ty::Named(n) => n.clone(),
        other => format!("{other:?}").split('(').next().unwrap_or("?").to_string(),
    }
}

fn to_json_untyped(v: &V) -> Json {
    match v {
        V::Int(n) => i64::try_from(*n).map(Json::from).unwrap_or_else(|_| Json::String(n.to_string())),
        V::Float(f) => json!(f),
        V::Bool(b) => json!(b),
        V::Text(s) => json!(s),
        V::Bytes(b) => json!(base64::engine::general_purpose::STANDARD.encode(b)),
        V::Unit => Json::Null,
        V::List(xs) => Json::Array(xs.iter().map(to_json_untyped).collect()),
        V::Maybe(None) => Json::Null,
        V::Maybe(Some(x)) => to_json_untyped(x),
        V::Either(false, x) => json!({ "Left": to_json_untyped(x) }),
        V::Either(true, x) => json!({ "Right": to_json_untyped(x) }),
        V::Data(c, fields) => json!({ c.clone(): Json::Object(fields.iter().map(|(n, v)| (n.clone(), to_json_untyped(v))).collect()) }),
    }
}

// ---------------------------------------------------------------------------
// Encodings
// ---------------------------------------------------------------------------

/// How a value is written into a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// JSON: a product as an object of its fields; a sum's constructor
    /// without fields as its name, with fields as `{"Name": {fields}}`;
    /// `Maybe` as the value or null; `Either` as `{"Left": v}` or
    /// `{"Right": v}`; bytes as base64 text.
    Json,
    /// LawSpec's wire encoding, the one values cross the network in.
    LawSpec,
}

impl std::str::FromStr for Format {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "json" => Ok(Format::Json),
            "lawspec" => Ok(Format::LawSpec),
            other => Err(format!("{other} is not a value format: json or lawspec")),
        }
    }
}

impl Types {
    fn ctors_of<'a>(&'a self, t: &'a Ty) -> Option<(&'a String, &'a Vec<Ctor>)> {
        match t {
            Ty::Named(n) => match self.refinements.get(n) {
                Some(r) => self.ctors_of(r),
                None => self.data.get_key_value(n),
            },
            Ty::Refined(_, b, _) => self.ctors_of(b),
            _ => None,
        }
    }

    fn base<'a>(&'a self, t: &'a Ty) -> &'a Ty {
        match t {
            Ty::Refined(_, b, _) => self.base(b),
            Ty::Named(n) => match self.refinements.get(n) {
                Some(r) => self.base(r),
                None => t,
            },
            _ => t,
        }
    }

    /// A value as JSON.
    pub fn to_json(&self, t: &Ty, v: &V) -> Json {
        let t = self.base(t);
        match (t, v) {
            (Ty::Maybe(a), V::Maybe(Some(x))) => self.to_json(a, x),
            (Ty::List(a), V::List(xs)) => Json::Array(xs.iter().map(|x| self.to_json(a, x)).collect()),
            (Ty::Either(l, _), V::Either(false, x)) => json!({ "Left": self.to_json(l, x) }),
            (Ty::Either(_, r), V::Either(true, x)) => json!({ "Right": self.to_json(r, x) }),
            (Ty::Named(_), V::Data(c, fields)) => {
                let ctors = self.ctors_of(t).map(|(_, c)| c.clone()).unwrap_or_default();
                let ctor = ctors.iter().find(|x| &x.name == c);
                let obj = Json::Object(
                    fields
                        .iter()
                        .map(|(n, fv)| {
                            let ft = ctor.and_then(|k| k.fields.iter().find(|(x, _)| x == n)).map(|(_, t)| t.clone()).unwrap_or(Ty::Unit);
                            (n.clone(), self.to_json(&ft, fv))
                        })
                        .collect(),
                );
                if ctors.len() == 1 {
                    obj
                } else if fields.is_empty() {
                    Json::String(c.clone())
                } else {
                    json!({ c.clone(): obj })
                }
            }
            (_, v) => to_json_untyped(v),
        }
    }

    /// A value read from JSON, as a type says to read it.
    pub fn from_json(&self, t: &Ty, j: &Json, path: &str) -> Result<V, String> {
        let t = self.base(t);
        let bad = || format!("{path}: {j} is not a {}", type_name(t));
        Ok(match t {
            Ty::Int(..) | Ty::Integer => match j {
                Json::Number(n) if n.is_i64() || n.is_u64() => V::Int(n.as_i64().map(i128::from).unwrap_or_else(|| n.as_u64().unwrap_or(0) as i128)),
                Json::String(s) => V::Int(s.parse().map_err(|_| bad())?),
                _ => return Err(bad()),
            },
            Ty::Float => V::Float(j.as_f64().ok_or_else(bad)?),
            Ty::Bool => V::Bool(j.as_bool().ok_or_else(bad)?),
            Ty::Text => V::Text(j.as_str().ok_or_else(bad)?.to_string()),
            Ty::Bytes => V::Bytes(base64::engine::general_purpose::STANDARD.decode(j.as_str().ok_or_else(bad)?).map_err(|_| bad())?),
            Ty::Unit => V::Unit,
            Ty::Maybe(a) => match j {
                Json::Null => V::Maybe(None),
                x => V::Maybe(Some(Box::new(self.from_json(a, x, path)?))),
            },
            Ty::List(a) => V::List(
                j.as_array().ok_or_else(bad)?.iter().enumerate().map(|(i, x)| self.from_json(a, x, &format!("{path}[{i}]"))).collect::<Result<_, _>>()?,
            ),
            Ty::Either(l, r) => match (j.get("Left"), j.get("Right")) {
                (Some(x), None) => V::Either(false, Box::new(self.from_json(l, x, path)?)),
                (None, Some(x)) => V::Either(true, Box::new(self.from_json(r, x, path)?)),
                _ => return Err(bad()),
            },
            Ty::Named(n) => {
                let ctors = self.data.get(n).ok_or_else(|| format!("{path}: no type {n}"))?;
                let (ctor, obj) = if ctors.len() == 1 {
                    (&ctors[0], j.clone())
                } else {
                    match j {
                        Json::String(c) => (ctors.iter().find(|x| &x.name == c).ok_or_else(|| format!("{path}: {c} is not a constructor of {n}"))?, json!({})),
                        Json::Object(m) if m.len() == 1 => {
                            let (c, o) = m.iter().next().unwrap();
                            (ctors.iter().find(|x| &x.name == c).ok_or_else(|| format!("{path}: {c} is not a constructor of {n}"))?, o.clone())
                        }
                        _ => return Err(bad()),
                    }
                };
                let m = obj.as_object().ok_or_else(bad)?;
                let mut fields = vec![];
                for (f, ft) in &ctor.fields {
                    let x = m.get(f).ok_or_else(|| format!("{path}.{f}: missing"))?;
                    fields.push((f.clone(), self.from_json(ft, x, &format!("{path}.{f}"))?));
                }
                V::Data(ctor.name.clone(), fields)
            }
            Ty::Refined(..) => unreachable!("base types are not refined"),
        })
    }

    /// A value in LawSpec's wire encoding.
    pub fn to_lawspec(&self, t: &Ty, v: &V, out: &mut Vec<u8>) {
        let t = self.base(t);
        match (t, v) {
            (_, V::Int(n)) => leb(out, zigzag(*n)),
            (_, V::Bool(b)) => out.push(*b as u8),
            (_, V::Text(s)) => {
                leb(out, s.len() as u128);
                out.extend_from_slice(s.as_bytes());
            }
            (_, V::Bytes(b)) => {
                leb(out, b.len() as u128);
                out.extend_from_slice(b);
            }
            (_, V::Float(f)) => out.extend_from_slice(&f.to_le_bytes()),
            (_, V::Unit) => {}
            (Ty::List(a), V::List(xs)) => {
                leb(out, xs.len() as u128);
                for x in xs {
                    self.to_lawspec(a, x, out);
                }
            }
            (_, V::Maybe(None)) => out.push(0),
            (Ty::Maybe(a), V::Maybe(Some(x))) => {
                out.push(1);
                self.to_lawspec(a, x, out);
            }
            (Ty::Either(l, r), V::Either(right, x)) => {
                out.push(*right as u8);
                self.to_lawspec(if *right { r } else { l }, x, out);
            }
            (_, V::Data(c, fields)) => {
                let ctors = self.ctors_of(t).map(|(_, c)| c.clone()).unwrap_or_default();
                let i = ctors.iter().position(|x| &x.name == c).unwrap_or(0);
                leb(out, i as u128);
                for (n, fv) in fields {
                    let ft = ctors.get(i).and_then(|k| k.fields.iter().find(|(x, _)| x == n)).map(|(_, t)| t.clone()).unwrap_or(Ty::Unit);
                    self.to_lawspec(&ft, fv, out);
                }
            }
            _ => {}
        }
    }

    /// A value read from LawSpec's wire encoding.
    pub fn from_lawspec(&self, t: &Ty, b: &[u8], at: &mut usize, path: &str) -> Result<V, String> {
        let t = self.base(t);
        let end = || format!("{path}: the encoding ends early");
        Ok(match t {
            Ty::Int(..) | Ty::Integer => V::Int(unzigzag(read_leb(b, at).ok_or_else(end)?)),
            Ty::Bool => {
                let x = *b.get(*at).ok_or_else(end)?;
                *at += 1;
                V::Bool(x == 1)
            }
            Ty::Text | Ty::Bytes => {
                let n = read_leb(b, at).ok_or_else(end)? as usize;
                let s = b.get(*at..*at + n).ok_or_else(end)?.to_vec();
                *at += n;
                if *t == Ty::Text {
                    V::Text(String::from_utf8(s).map_err(|_| format!("{path}: text that is not UTF-8"))?)
                } else {
                    V::Bytes(s)
                }
            }
            Ty::Float => {
                let s = b.get(*at..*at + 8).ok_or_else(end)?;
                *at += 8;
                V::Float(f64::from_le_bytes(s.try_into().unwrap()))
            }
            Ty::Unit => V::Unit,
            Ty::List(a) => {
                let n = read_leb(b, at).ok_or_else(end)? as usize;
                V::List((0..n.min(1 << 20)).map(|i| self.from_lawspec(a, b, at, &format!("{path}[{i}]"))).collect::<Result<_, _>>()?)
            }
            Ty::Maybe(a) => {
                let tag = *b.get(*at).ok_or_else(end)?;
                *at += 1;
                if tag == 0 { V::Maybe(None) } else { V::Maybe(Some(Box::new(self.from_lawspec(a, b, at, path)?))) }
            }
            Ty::Either(l, r) => {
                let tag = *b.get(*at).ok_or_else(end)?;
                *at += 1;
                V::Either(tag == 1, Box::new(self.from_lawspec(if tag == 1 { r } else { l }, b, at, path)?))
            }
            Ty::Named(n) => {
                let ctors = self.data.get(n).ok_or_else(|| format!("{path}: no type {n}"))?;
                let i = read_leb(b, at).ok_or_else(end)? as usize;
                let ctor = ctors.get(i).ok_or_else(|| format!("{path}: constructor {i} of {n} does not exist"))?;
                let mut fields = vec![];
                for (f, ft) in &ctor.fields {
                    fields.push((f.clone(), self.from_lawspec(ft, b, at, &format!("{path}.{f}"))?));
                }
                V::Data(ctor.name.clone(), fields)
            }
            Ty::Refined(..) => unreachable!("base types are not refined"),
        })
    }

    /// A value encoded as a record's value.
    pub fn encode(&self, t: &Ty, v: &V, format: Format) -> Vec<u8> {
        match format {
            Format::Json => serde_json::to_vec(&self.to_json(t, v)).unwrap_or_default(),
            Format::LawSpec => {
                let mut out = vec![];
                self.to_lawspec(t, v, &mut out);
                out
            }
        }
    }

    /// A record's value decoded and checked against a type: every way it is
    /// not a value of the type.
    pub fn validate(&self, t: &Ty, bytes: &[u8], format: Format) -> Vec<String> {
        let decoded = match format {
            Format::Json => serde_json::from_slice::<Json>(bytes).map_err(|e| format!("value: not JSON: {e}")).and_then(|j| self.from_json(t, &j, "value")),
            Format::LawSpec => {
                let mut at = 0;
                self.from_lawspec(t, bytes, &mut at, "value").and_then(|v| {
                    if at == bytes.len() { Ok(v) } else { Err(format!("value: {} bytes after the value", bytes.len() - at)) }
                })
            }
        };
        match decoded {
            Ok(v) => self.check(t, &v, "value"),
            Err(e) => vec![e],
        }
    }
}

fn zigzag(n: i128) -> u128 {
    ((n << 1) ^ (n >> 127)) as u128
}

fn unzigzag(n: u128) -> i128 {
    ((n >> 1) as i128) ^ -((n & 1) as i128)
}

fn leb(out: &mut Vec<u8>, mut n: u128) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_leb(b: &[u8], at: &mut usize) -> Option<u128> {
    let mut n: u128 = 0;
    let mut shift = 0;
    loop {
        let byte = *b.get(*at)?;
        *at += 1;
        n |= ((byte & 0x7f) as u128) << shift;
        if byte & 0x80 == 0 {
            return Some(n);
        }
        shift += 7;
        if shift > 126 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"
unit shop.orders

-- An order as the shop publishes it.
type Order is
  Order
    id :: (v :: Text where v matches regex "ord-[0-9]{4}")
    amount :: (v :: Int64 where v > 0 && v <= 100000)
    currency :: Currency
    note :: Maybe Text
    lines :: (v :: List Line where prelude.length v >= 1 && prelude.length v <= 3)
end

type Line is
  Line
    sku :: Sku
    quantity :: (q :: Int32 where q >= 1)
end

type Currency is | EUR | USD | GBP end

refinement Sku is (s :: Text where validSku s) end

definition validSku (s :: Text) :: Bool is prelude.length s == 6 end

law `something` is
  definition is `for all` (x :: Int8) . x == x end
end
"#;

    fn types() -> Types {
        Types::read(&[SPEC.to_string()])
    }

    #[test]
    fn reads_types_refinements_and_definitions() {
        let t = types();
        assert_eq!(t.data["Order"][0].fields.len(), 5);
        assert_eq!(t.data["Currency"].len(), 3);
        assert!(t.refinements.contains_key("Sku"));
        assert!(t.defs.contains_key("validSku"));
    }

    #[test]
    fn generated_values_satisfy_their_type_in_both_formats() {
        let t = types();
        let ty = t.parse_type("Order").unwrap();
        let mut rng = Rng::new(7);
        for _ in 0..200 {
            let v = t.generate(&ty, &mut rng).unwrap();
            assert!(t.check(&ty, &v, "value").is_empty(), "{v:?}");
            for format in [Format::Json, Format::LawSpec] {
                let bytes = t.encode(&ty, &v, format);
                assert!(t.validate(&ty, &bytes, format).is_empty(), "{format:?} {v:?}");
            }
        }
    }

    #[test]
    fn values_outside_the_type_are_reported_with_their_path() {
        let t = types();
        let ty = t.parse_type("Order").unwrap();
        let bad = br#"{"id":"ord-12","amount":0,"currency":"JPY","note":null,"lines":[{"sku":"abc","quantity":1}]}"#;
        let found = t.validate(&ty, bad, Format::Json);
        assert!(found.iter().any(|f| f.starts_with("value.currency") && f.contains("JPY")), "{found:?}");
        let bad = br#"{"id":"ord-12","amount":0,"currency":"EUR","note":null,"lines":[{"sku":"abc","quantity":1}]}"#;
        let found = t.validate(&ty, bad, Format::Json);
        assert!(found.iter().any(|f| f.starts_with("value.id")), "{found:?}");
        assert!(found.iter().any(|f| f.starts_with("value.amount")), "{found:?}");
        assert!(found.iter().any(|f| f.starts_with("value.lines[0].sku")), "{found:?}");
        assert!(!t.validate(&ty, b"not json", Format::Json).is_empty());
    }

    #[test]
    fn the_same_seed_gives_the_same_values() {
        let t = types();
        let ty = t.parse_type("List Order").unwrap();
        let a = t.generate(&ty, &mut Rng::new(42)).unwrap();
        let b = t.generate(&ty, &mut Rng::new(42)).unwrap();
        assert_eq!(a, b);
    }
}
