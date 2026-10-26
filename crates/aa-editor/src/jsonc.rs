//! The server's spawn files are JSON arrays with `//` comments (`"UnitId": 3111, // Gorgon`). Saving must not lose
//! the comments of the thousands of entries the editor did not touch, so a file is split into the source text of its
//! top-level elements: untouched elements are written back byte for byte.

/// Source spans (`start..end`) of the elements of the top-level array in `text`.
pub fn array_elements(text: &str) -> Result<Vec<(usize, usize)>, String> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut last_end = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'"' => {
                if depth == 1 && start.is_none() {
                    start = Some(i);
                }
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                last_end = i + 1;
            }
            b'[' | b'{' => {
                if depth == 0 && c != b'[' {
                    return Err("no top-level array".into());
                }
                if depth == 1 && start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b']' | b'}' => {
                if depth == 0 {
                    return Err(format!("unbalanced '{}' at byte {i}", c as char));
                }
                depth -= 1;
                if depth == 1 {
                    last_end = i + 1;
                }
                if depth == 0 {
                    if let Some(s) = start.take() {
                        out.push((s, last_end));
                    }
                    return Ok(out);
                }
            }
            b',' if depth == 1 => {
                if let Some(s) = start.take() {
                    out.push((s, last_end));
                }
            }
            c if depth == 1 && !c.is_ascii_whitespace() => {
                // a bare number / literal element
                if start.is_none() {
                    start = Some(i);
                }
                last_end = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    Err("no top-level array".into())
}

/// `text` without `//` and `/* */` comments (strings kept intact), for `serde_json`.
pub fn strip_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                out.push(b'"');
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out.push(b[i]);
                        i += 1;
                    }
                    out.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    out.push(b'"');
                }
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"[
    {
        "UnitId": 3111, // Gorgon, "quoted" [x]
        "Position": { "X": 1.5, "Y": 2, "Z": 3 }
    },
    /* block */ { "UnitId": 2, "Title": "a, b } ]", "Position": {} }
]
"#;

    #[test]
    fn splits_elements_with_comments_and_tricky_strings() {
        let spans = array_elements(TEXT).unwrap();
        assert_eq!(spans.len(), 2);
        let first = &TEXT[spans[0].0..spans[0].1];
        assert!(first.starts_with('{') && first.ends_with('}') && first.contains("// Gorgon"));
        let second = &TEXT[spans[1].0..spans[1].1];
        assert_eq!(second, r#"{ "UnitId": 2, "Title": "a, b } ]", "Position": {} }"#);
        for (s, e) in spans {
            let v: serde_json::Value = serde_json::from_str(&strip_comments(&TEXT[s..e])).unwrap();
            assert!(v.get("UnitId").is_some());
        }
    }

    #[test]
    fn empty_array() {
        assert!(array_elements("[ ]").unwrap().is_empty());
        assert!(array_elements("{}").is_err());
    }
}
