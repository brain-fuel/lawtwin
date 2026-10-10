//! Names: OpenAPI's names made into LawSpec identifiers that every LawSpec
//! target accepts, and kept unique.

use std::collections::BTreeSet;

/// Words a lower-case LawSpec name must not be: LawSpec's own keywords, and
/// the reserved words of the targets a contract is generated for (Rust,
/// Python, Go, Java, Kotlin, JavaScript, TypeScript, Haskell). A field or
/// parameter named one of them gets a trailing underscore.
const RESERVED: &[&str] = &[
    "ability",
    "abstract",
    "acquire",
    "all",
    "and",
    "any",
    "are",
    "as",
    "assert",
    "async",
    "await",
    "boolean",
    "break",
    "by",
    "byte",
    "case",
    "catch",
    "chan",
    "char",
    "class",
    "companion",
    "const",
    "continue",
    "crate",
    "data",
    "def",
    "default",
    "defer",
    "definition",
    "del",
    "delete",
    "description",
    "do",
    "double",
    "dyn",
    "elif",
    "else",
    "end",
    "enum",
    "error",
    "example",
    "except",
    "expect",
    "export",
    "extends",
    "external",
    "fails",
    "fallthrough",
    "false",
    "final",
    "finally",
    "float",
    "fn",
    "for",
    "foreign",
    "from",
    "fun",
    "func",
    "function",
    "global",
    "go",
    "goto",
    "handle",
    "handler",
    "handlers",
    "if",
    "impl",
    "implements",
    "implies",
    "import",
    "in",
    "infix",
    "init",
    "inner",
    "instance",
    "instanceof",
    "int",
    "interface",
    "is",
    "lambda",
    "law",
    "laws",
    "let",
    "long",
    "loop",
    "map",
    "match",
    "mod",
    "model",
    "module",
    "move",
    "mut",
    "native",
    "new",
    "newtype",
    "nil",
    "none",
    "nonlocal",
    "not",
    "null",
    "object",
    "of",
    "open",
    "or",
    "override",
    "package",
    "pass",
    "private",
    "protected",
    "pub",
    "public",
    "qualified",
    "raise",
    "range",
    "rationale",
    "record",
    "ref",
    "references",
    "refinement",
    "requires",
    "release",
    "resource",
    "return",
    "sealed",
    "select",
    "self",
    "short",
    "start",
    "state",
    "static",
    "strictfp",
    "string",
    "struct",
    "super",
    "switch",
    "synchronized",
    "then",
    "this",
    "throw",
    "throws",
    "trait",
    "transient",
    "true",
    "try",
    "type",
    "typealias",
    "typeof",
    "unit",
    "unsafe",
    "use",
    "uses",
    "val",
    "var",
    "void",
    "volatile",
    "when",
    "where",
    "while",
    "with",
    "wrapper",
    "yield",
];

/// Upper-case names LawSpec or its prelude already gives a meaning.
const RESERVED_TYPES: &[&str] = &[
    "Bool",
    "Bytes",
    "Char",
    "Deque",
    "Double",
    "Either",
    "Entry",
    "Float32",
    "Float64",
    "Int16",
    "Int32",
    "Int64",
    "Int8",
    "Integer",
    "KeyVal",
    "Left",
    "List",
    "Maybe",
    "Natural",
    "Nothing",
    "Just",
    "Ordering",
    "Pair",
    "Queue",
    "Right",
    "Set",
    "Stack",
    "Text",
    "UInt16",
    "UInt32",
    "UInt64",
    "UInt8",
    "Unit",
    "Nil",
    "Cons",
    "Fail",
    "Clock",
    "Random",
    "Option",
    "Result",
    "String",
    "Object",
    "Some",
    "None",
    "Self",
    "Type",
    "Value",
    "Error",
    "Exception",
];

/// The words of a name: split at anything that is not a letter or digit, and
/// where a lower-case letter or digit meets an upper-case one.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_ascii_alphanumeric() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        let boundary = c.is_ascii_uppercase()
            && i > 0
            && (chars[i - 1].is_ascii_lowercase()
                || chars[i - 1].is_ascii_digit()
                || (chars[i - 1].is_ascii_uppercase()
                    && chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase())));
        if boundary && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn capitalised(word: &str) -> String {
    let mut cs = word.chars();
    match cs.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + &cs.as_str().to_ascii_lowercase(),
        None => String::new(),
    }
}

/// `node_id` and `NodeID` to `NodeId`; a name that would start with a digit
/// starts with `N`.
pub fn upper_camel(name: &str) -> String {
    let joined: String = words(name).iter().map(|w| capitalised(w)).collect();
    if joined.is_empty() {
        "Unnamed".to_string()
    } else if joined.starts_with(|c: char| c.is_ascii_digit()) {
        format!("N{joined}")
    } else {
        joined
    }
}

/// `node_id` to `nodeId`; a name that would start with a digit starts with `n`.
pub fn lower_camel(name: &str) -> String {
    let upper = upper_camel(name);
    let mut cs = upper.chars();
    let first = cs.next().unwrap().to_ascii_lowercase();
    format!("{first}{}", cs.as_str())
}

/// A field, parameter or operation name.
pub fn value_name(name: &str) -> String {
    let n = lower_camel(name);
    if RESERVED.contains(&n.as_str()) {
        format!("{n}_")
    } else {
        n
    }
}

/// Names already taken in one namespace, giving each new one a free variant.
#[derive(Default)]
pub struct Namespace {
    taken: BTreeSet<String>,
}

impl Namespace {
    /// A namespace in which LawSpec's own type names are taken.
    pub fn with_reserved_types() -> Self {
        Namespace {
            taken: RESERVED_TYPES.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The name itself when free, otherwise the name with a suffix, then a number.
    pub fn claim(&mut self, wanted: &str, suffix: &str) -> String {
        if self.taken.insert(wanted.to_string()) {
            return wanted.to_string();
        }
        let with_suffix = format!("{wanted}{suffix}");
        if self.taken.insert(with_suffix.clone()) {
            return with_suffix;
        }
        (2..)
            .map(|n| format!("{wanted}{suffix}{n}"))
            .find(|c| self.taken.insert(c.clone()))
            .unwrap()
    }

    /// Take a name without asking.
    pub fn reserve(&mut self, name: &str) {
        self.taken.insert(name.to_string());
    }

    /// Whether the name is taken.
    pub fn contains(&self, name: &str) -> bool {
        self.taken.contains(name)
    }
}

/// A LawSpec text literal.
pub fn text_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A one-line comment from a description: the first sentence, at most 100 characters.
pub fn summary(description: &str) -> String {
    let line = description
        .split(['\n', '\r'])
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let sentence = match line.find(". ") {
        Some(i) => &line[..=i],
        None => line,
    };
    let mut s: String = sentence.chars().filter(|c| !c.is_control()).collect();
    if s.chars().count() > 100 {
        s = s.chars().take(97).collect::<String>() + "...";
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names are split into words and joined in camel case.
    #[test]
    fn camel_cases() {
        assert_eq!(upper_camel("nullable-simple-user"), "NullableSimpleUser");
        assert_eq!(upper_camel("ApiResponse"), "ApiResponse");
        assert_eq!(upper_camel("HTTPServer"), "HttpServer");
        assert_eq!(lower_camel("node_id"), "nodeId");
        assert_eq!(lower_camel("issues/create-label"), "issuesCreateLabel");
        assert_eq!(value_name("default"), "default_");
        assert_eq!(value_name("type"), "type_");
        assert_eq!(upper_camel("2fa"), "N2fa");
    }

    /// A taken name gets the suffix, then a number.
    #[test]
    fn namespace_suffixes() {
        let mut ns = Namespace::with_reserved_types();
        assert_eq!(ns.claim("Pet", "Schema"), "Pet");
        assert_eq!(ns.claim("Pet", "Schema"), "PetSchema");
        assert_eq!(ns.claim("Pet", "Schema"), "PetSchema2");
        assert_eq!(ns.claim("Error", "Schema"), "ErrorSchema");
    }
}
