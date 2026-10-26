//! Lua 5.1 syntax colouring for the script editor (and `toc.g`: `#` comments).

use bevy_egui::egui::{Color32, FontId, TextFormat, text::LayoutJob};

const KEYWORDS: [&str; 21] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or",
    "repeat", "return", "then", "true", "until", "while",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
    /// A widget API call (`:AddAnchor`, `:SetExtent`, `CreateChildWidget` ...).
    Method,
    Global,
}

fn colour(t: Tok) -> Color32 {
    match t {
        Tok::Plain => Color32::from_rgb(212, 212, 212),
        Tok::Keyword => Color32::from_rgb(197, 134, 192),
        Tok::String => Color32::from_rgb(206, 145, 120),
        Tok::Comment => Color32::from_rgb(106, 153, 85),
        Tok::Number => Color32::from_rgb(181, 206, 168),
        Tok::Method => Color32::from_rgb(220, 220, 170),
        Tok::Global => Color32::from_rgb(78, 201, 176),
    }
}

/// Byte ranges and kinds of the tokens of `src` (everything is covered, in order).
pub fn tokens(src: &str, toc: bool) -> Vec<(usize, usize, Tok)> {
    let b = src.as_bytes();
    let mut out: Vec<(usize, usize, Tok)> = Vec::new();
    let mut push = |s: usize, e: usize, t: Tok| {
        if let Some(last) = out.last_mut()
            && last.2 == t
            && last.1 == s
        {
            last.1 = e;
            return;
        }
        out.push((s, e, t));
    };
    let mut i = 0;
    if toc {
        while i < b.len() {
            let e = src[i..].find('\n').map(|n| i + n + 1).unwrap_or(b.len());
            push(i, e, if b[i] == b'#' { Tok::Comment } else { Tok::Plain });
            i = e;
        }
        return out;
    }
    let long_bracket = |i: usize| -> Option<usize> {
        // `[` `=`* `[` -> level
        if b.get(i) != Some(&b'[') {
            return None;
        }
        let mut j = i + 1;
        while b.get(j) == Some(&b'=') {
            j += 1;
        }
        (b.get(j) == Some(&b'[')).then_some(j - i - 1)
    };
    let close = |from: usize, level: usize| -> usize {
        let pat = format!("]{}]", "=".repeat(level));
        src[from..].find(&pat).map(|n| from + n + pat.len()).unwrap_or(b.len())
    };
    while i < b.len() {
        let c = b[i];
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            let e = match long_bracket(i + 2) {
                Some(level) => close(i + 4 + level, level),
                None => src[i..].find('\n').map(|n| i + n).unwrap_or(b.len()),
            };
            push(i, e, Tok::Comment);
            i = e;
        } else if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < b.len() && b[j] != c && b[j] != b'\n' {
                if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let e = (j + 1).min(b.len());
            push(i, e, Tok::String);
            i = e;
        } else if let Some(level) = long_bracket(i) {
            let e = close(i + 2 + level, level);
            push(i, e, Tok::String);
            i = e;
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || ((b[j] == b'-' || b[j] == b'+') && matches!(b[j - 1], b'e' | b'E'))) {
                j += 1;
            }
            push(i, j, Tok::Number);
            i = j;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            let word = &src[i..j];
            let prev = src[..i].trim_end().as_bytes().last().copied();
            let t = if KEYWORDS.contains(&word) {
                Tok::Keyword
            } else if prev == Some(b':') || word.starts_with("Create") {
                Tok::Method
            } else if word.starts_with("X2") || word == "UIParent" || word == "ADDON" {
                Tok::Global
            } else {
                Tok::Plain
            };
            push(i, j, t);
            i = j;
        } else {
            let len = src[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            push(i, i + len, Tok::Plain);
            i += len;
        }
    }
    out
}

/// The coloured layout of `src` for a `TextEdit` layouter.
pub fn layout(src: &str, toc: bool, font: FontId, highlight: Option<(usize, usize)>) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (s, e, t) in tokens(src, toc) {
        let mut fmt = TextFormat::simple(font.clone(), colour(t));
        if let Some((hs, he)) = highlight
            && s < he
            && e > hs
        {
            // split around the highlighted (found) range
            let (a, z) = (s.max(hs), e.min(he));
            if s < a {
                job.append(&src[s..a], 0.0, fmt.clone());
            }
            let mut hl = fmt.clone();
            hl.background = Color32::from_rgb(98, 80, 20);
            job.append(&src[a..z], 0.0, hl);
            if z < e {
                job.append(&src[z..e], 0.0, fmt);
            }
            continue;
        }
        if t == Tok::Comment {
            fmt.italics = true;
        }
        job.append(&src[s..e], 0.0, fmt);
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_everything() {
        let src = "local w = UIParent:CreateWidget(\"window\", 'x', 0) -- c\n--[[ long\n]] x = [[s]] .. 1.5e-3\n";
        let toks = tokens(src, false);
        let mut at = 0;
        for &(s, e, _) in &toks {
            assert_eq!(s, at);
            at = e;
        }
        assert_eq!(at, src.len());
        let kinds: Vec<Tok> = toks.iter().map(|t| t.2).collect();
        assert!(kinds.contains(&Tok::Comment) && kinds.contains(&Tok::String) && kinds.contains(&Tok::Method));
        assert!(toks.iter().any(|&(s, e, t)| t == Tok::Number && &src[s..e] == "1.5e-3"));
    }
}
