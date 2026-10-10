//! LawSpec type expressions as the wire map writes them, such as `Int64`,
//! `Maybe (List Tag)` or `KeyVal Text Int32`.

use std::fmt;

/// A LawSpec type of a contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Text,
    Int32,
    Int64,
    Float64,
    Bool,
    Bytes,
    Unit,
    Maybe(Box<Ty>),
    List(Box<Ty>),
    KeyVal(Box<Ty>, Box<Ty>),
    /// A record, enum or union of the contract, by name.
    Named(String),
}

impl Ty {
    /// Reads a type expression.
    pub fn parse(text: &str) -> Result<Ty, String> {
        let tokens = tokenize(text);
        let mut pos = 0;
        let ty = app(&tokens, &mut pos).map_err(|e| format!("the type `{text}`: {e}"))?;
        if pos != tokens.len() {
            return Err(format!("the type `{text}` has more after it"));
        }
        Ok(ty)
    }

    /// The type inside `Maybe`, or the type itself.
    pub fn unmaybe(&self) -> &Ty {
        match self {
            Ty::Maybe(t) => t,
            t => t,
        }
    }

    /// Whether it is `Maybe` of something.
    pub fn is_maybe(&self) -> bool {
        matches!(self, Ty::Maybe(_))
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn atom(t: &Ty) -> String {
            match t {
                Ty::Maybe(_) | Ty::List(_) | Ty::KeyVal(..) => format!("({t})"),
                _ => t.to_string(),
            }
        }
        match self {
            Ty::Text => write!(f, "Text"),
            Ty::Int32 => write!(f, "Int32"),
            Ty::Int64 => write!(f, "Int64"),
            Ty::Float64 => write!(f, "Float64"),
            Ty::Bool => write!(f, "Bool"),
            Ty::Bytes => write!(f, "Bytes"),
            Ty::Unit => write!(f, "Unit"),
            Ty::Maybe(t) => write!(f, "Maybe {}", atom(t)),
            Ty::List(t) => write!(f, "List {}", atom(t)),
            Ty::KeyVal(k, v) => write!(f, "KeyVal {} {}", atom(k), atom(v)),
            Ty::Named(n) => write!(f, "{n}"),
        }
    }
}

fn tokenize(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut word = String::new();
    for c in text.chars() {
        match c {
            '(' | ')' => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                out.push(c.to_string());
            }
            c if c.is_whitespace() => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

fn atom(tokens: &[String], pos: &mut usize) -> Result<Ty, String> {
    let t = tokens.get(*pos).ok_or("a type is missing")?;
    *pos += 1;
    if t == "(" {
        let ty = app(tokens, pos)?;
        if tokens.get(*pos).map(String::as_str) != Some(")") {
            return Err("a ( is not closed".into());
        }
        *pos += 1;
        return Ok(ty);
    }
    if t == ")" {
        return Err("a ) without a (".into());
    }
    Ok(match t.as_str() {
        "Text" => Ty::Text,
        "Int32" => Ty::Int32,
        "Int64" => Ty::Int64,
        "Float64" => Ty::Float64,
        "Bool" => Ty::Bool,
        "Bytes" => Ty::Bytes,
        "Unit" => Ty::Unit,
        "Maybe" => Ty::Maybe(Box::new(atom(tokens, pos)?)),
        "List" => Ty::List(Box::new(atom(tokens, pos)?)),
        "KeyVal" => {
            let k = atom(tokens, pos)?;
            Ty::KeyVal(Box::new(k), Box::new(atom(tokens, pos)?))
        }
        name => Ty::Named(name.to_string()),
    })
}

fn app(tokens: &[String], pos: &mut usize) -> Result<Ty, String> {
    atom(tokens, pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type expressions read back as written.
    #[test]
    fn parses_and_prints() {
        for t in ["Int64", "Maybe (List Tag)", "KeyVal Text Int32", "List (Maybe Text)", "KeyVal LabelKey Label"] {
            assert_eq!(Ty::parse(t).unwrap().to_string(), t);
        }
        assert!(Ty::parse("Maybe").is_err());
        assert!(Ty::parse("(Text").is_err());
    }
}
