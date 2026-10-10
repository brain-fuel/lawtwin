//! OpenAPI patterns as LawSpec regexes.
//!
//! An OpenAPI `pattern` is an ECMA-262 regex that may match anywhere in the
//! text. A LawSpec regex is portable (the subset that means the same in RE2
//! and ECMAScript) and matches the whole text. A pattern is converted by
//! dropping its anchors and padding an unanchored end with `.*`; a pattern
//! that uses what the portable dialect leaves out (lookaround,
//! backreferences, word boundaries, Unicode classes, lazy quantifiers,
//! flags) is not converted, and the importer says so.

/// The regex a format's values match, for the formats with a fixed syntax.
pub fn format_regex(format: &str) -> Option<&'static str> {
    Some(match format {
        "date-time" => r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+\-]\d{2}:\d{2})",
        "date" => r"\d{4}-\d{2}-\d{2}",
        "time" => r"\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+\-]\d{2}:\d{2})?",
        "uuid" => r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        "email" => r"[^@\s]+@[^@\s]+",
        "uri" | "url" => r"[A-Za-z][A-Za-z0-9+.\-]*:\S*",
        "ipv4" => r"(?:\d{1,3}\.){3}\d{1,3}",
        "hostname" => r"[A-Za-z0-9](?:[A-Za-z0-9.\-]*[A-Za-z0-9])?",
        "byte" => r"(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?",
        _ => return None,
    })
}

/// The pattern as a whole-text portable regex, or why it cannot be one.
pub fn portable(pattern: &str) -> Result<String, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut start = 0;
    let mut end = chars.len();
    let anchored_start = chars.first() == Some(&'^');
    if anchored_start {
        start = 1;
    }
    let anchored_end = end > start && chars[end - 1] == '$' && !escaped(&chars, end - 1);
    if anchored_end {
        end -= 1;
    }
    let body: Vec<char> = chars[start..end].to_vec();
    check(&body)?;
    let body: String = body.into_iter().collect();
    let mut out = String::new();
    if !anchored_start {
        out.push_str(".*");
    }
    if anchored_start && anchored_end && !body.contains('|') {
        out.push_str(&body);
    } else {
        out.push_str("(?:");
        out.push_str(&body);
        out.push(')');
    }
    if !anchored_end {
        out.push_str(".*");
    }
    Ok(out)
}

fn escaped(chars: &[char], i: usize) -> bool {
    let mut n = 0;
    let mut j = i;
    while j > 0 && chars[j - 1] == '\\' {
        n += 1;
        j -= 1;
    }
    n % 2 == 1
}

/// Reject what the portable dialect leaves out.
fn check(chars: &[char]) -> Result<(), String> {
    let mut i = 0;
    let mut in_class = false;
    let mut depth = 0i32;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let next = *chars.get(i + 1).ok_or("ends with a lone backslash")?;
                let allowed = "\\.^$|?*+()[]{}-/ntrfvdwsDWS";
                if !allowed.contains(next) {
                    return Err(format!("the escape \\{next} is not portable"));
                }
                i += 2;
                continue;
            }
            '[' if !in_class => {
                in_class = true;
                if chars.get(i + 1) == Some(&'^') {
                    i += 1;
                }
                // A ] first in a class is literal in ECMAScript but not RE2.
                if chars.get(i + 1) == Some(&']') {
                    return Err("a class that starts with ] is not portable".into());
                }
            }
            ']' if in_class => in_class = false,
            _ if in_class => {}
            '(' => {
                depth += 1;
                if chars.get(i + 1) == Some(&'?') && chars.get(i + 2) != Some(&':') {
                    return Err("lookaround, named groups and flags are not portable".into());
                }
            }
            ')' => depth -= 1,
            '^' | '$' => return Err(format!("an anchor {c} inside the pattern is not portable")),
            '*' | '+' | '?' | '}' => {
                if chars.get(i + 1) == Some(&'?') {
                    return Err("lazy quantifiers are not portable".into());
                }
                if c == '}' {
                    let open = chars[..i].iter().rposition(|&x| x == '{').unwrap_or(i);
                    let inner: String = chars[open + 1..i].iter().collect();
                    for bound in inner.split(',') {
                        if let Ok(n) = bound.trim().parse::<u32>() {
                            if n > 1000 {
                                return Err(format!("a repetition bound of {n} is above 1000"));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    if in_class {
        return Err("a class is never closed".into());
    }
    if depth != 0 {
        return Err("the groups do not balance".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_are_dropped_and_unanchored_ends_padded() {
        assert_eq!(portable("^[a-z]+$").unwrap(), "[a-z]+");
        assert_eq!(portable("^a|b$").unwrap(), "(?:a|b)");
        assert_eq!(portable("abc").unwrap(), ".*(?:abc).*");
        assert_eq!(portable("^abc").unwrap(), "(?:abc).*");
        assert_eq!(portable(r"^\d{3}\$$").unwrap(), r"\d{3}\$");
    }

    #[test]
    fn what_is_not_portable_is_refused() {
        assert!(portable("^(?=a)b$").is_err());
        assert!(portable(r"\bword\b").is_err());
        assert!(portable(r"(a)\1").is_err());
        assert!(portable("a+?").is_err());
        assert!(portable(r"\p{L}+").is_err());
        assert!(portable("a{1,5000}").is_err());
    }
}
