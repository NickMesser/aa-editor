//! Writing an inspector change back into the script source.
//!
//! A change knows the script line that last set the property (from [`crate::track`]): `button:SetExtent(80, 24)`.
//! When the arguments being changed are literals, they are replaced in place. Otherwise (a computed value) a new call
//! with the new literal values is inserted after the statement, with the same receiver expression; when there is no
//! recorded call at all, it goes after the line that created the object (`local w = parent:CreateChildWidget(...)`
//! gives the receiver `w`, a reflected `parent:CreateChildWidget("button", "ok", 0, true)` gives `parent.ok`).
//! Either way the result is shown as a preview first: a line inside a loop or a shared helper applies to every
//! object that runs it.

/// One property change.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Extent(f32, f32),
    /// `AddAnchor(point, <target as written>, rel, x, y)`.
    Anchor { point: String, rel: String, x: f32, y: f32 },
    Show(bool),
    Text(String),
    Alpha(f32),
    Color([f32; 4]),
    Texture(String),
    Coords([f32; 4]),
}

impl Edit {
    pub fn method(&self) -> &'static str {
        match self {
            Edit::Extent(..) => "SetExtent",
            Edit::Anchor { .. } => "AddAnchor",
            Edit::Show(_) => "Show",
            Edit::Text(_) => "SetText",
            Edit::Alpha(_) => "SetAlpha",
            Edit::Color(_) => "SetColor",
            Edit::Texture(_) => "SetTexture",
            Edit::Coords(_) => "SetCoords",
        }
    }

    /// The `track` property key whose site this change rewrites.
    pub fn key(&self) -> String {
        match self {
            Edit::Extent(..) => "extent".into(),
            Edit::Anchor { point, .. } => format!("anchor:{point}"),
            Edit::Show(_) => "show".into(),
            Edit::Text(_) => "text".into(),
            Edit::Alpha(_) => "alpha".into(),
            Edit::Color(_) => "color".into(),
            Edit::Texture(_) => "texture".into(),
            Edit::Coords(_) => "coords".into(),
        }
    }

    /// The new argument list; `old` are the arguments of the call being rewritten (for the anchor target).
    fn args(&self, old: &[String]) -> Option<String> {
        Some(match self {
            Edit::Extent(w, h) => format!("{}, {}", num(*w), num(*h)),
            Edit::Alpha(v) => num(*v),
            Edit::Anchor { point, rel, x, y } => {
                // keep the call's form: AddAnchor(point, [target], [relPoint], [x, y])
                let form = AnchorForm::parse(old);
                let mut a = vec![format!("\"{point}\"")];
                if let Some(t) = form.target {
                    a.push(t);
                }
                if form.rel || rel != point {
                    a.push(format!("\"{rel}\""));
                }
                a.push(num(*x));
                a.push(num(*y));
                a.join(", ")
            }
            Edit::Show(b) => b.to_string(),
            Edit::Text(s) => lua_string(s),
            Edit::Color(c) => c.iter().map(|v| num(*v)).collect::<Vec<_>>().join(", "),
            Edit::Texture(s) => lua_string(s),
            Edit::Coords(c) => c.iter().map(|v| num(*v)).collect::<Vec<_>>().join(", "),
        })
    }

    /// Whether the old arguments this change replaces are all literals (then it rewrites them in place).
    fn literal(&self, old: &[String]) -> bool {
        match self {
            Edit::Anchor { .. } => AnchorForm::parse(old).xy.is_none_or(|(x, y)| is_literal(&x) && is_literal(&y)),
            _ => !old.is_empty() && old.iter().all(|a| is_literal(a)),
        }
    }
}

const POINT_NAMES: [&str; 9] = ["TOPLEFT", "TOP", "TOPRIGHT", "LEFT", "CENTER", "RIGHT", "BOTTOMLEFT", "BOTTOM", "BOTTOMRIGHT"];

/// The shape of an `AddAnchor` call as written: `(point, [target], [relPoint], [x, y])`. Like the toolkit, a point
/// name string right after the point is the relative point (no target: the parent), a number starts the offsets,
/// anything else is the target.
struct AnchorForm {
    target: Option<String>,
    rel: bool,
    xy: Option<(String, String)>,
}

impl AnchorForm {
    fn parse(args: &[String]) -> AnchorForm {
        let is_point = |a: &str| {
            let t = a.trim();
            t.len() > 2
                && (t.starts_with('"') || t.starts_with('\''))
                && POINT_NAMES.contains(&t[1..t.len() - 1].to_ascii_uppercase().as_str())
        };
        let is_num = |a: &str| a.trim().parse::<f64>().is_ok();
        let mut i = 1;
        let mut target = None;
        if let Some(a) = args.get(i)
            && !is_point(a)
            && !is_num(a)
        {
            target = Some(a.clone());
            i += 1;
        }
        let rel = args.get(i).is_some_and(|a| is_point(a));
        if rel {
            i += 1;
        }
        let xy = (args.len() >= i + 2).then(|| (args[i].clone(), args[i + 1].clone()));
        AnchorForm { target, rel, xy }
    }
}

/// A number as a script would write it.
pub fn num(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e7 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn lua_string(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn is_literal(a: &str) -> bool {
    let a = a.trim();
    a.parse::<f64>().is_ok()
        || matches!(a, "true" | "false" | "nil")
        || (a.len() >= 2 && ((a.starts_with('"') && a.ends_with('"')) || (a.starts_with('\'') && a.ends_with('\''))))
        || (a.starts_with('-') && a[1..].trim().parse::<f64>().is_ok())
}

/// A parsed call in the source: byte offsets of the argument list (inside the parentheses), the arguments and the
/// receiver expression (`a.b[1]` of `a.b[1]:M(...)`), plus the byte offset of the statement's end of line.
#[derive(Debug)]
struct CallAt {
    args_start: usize,
    args_end: usize,
    args: Vec<String>,
    receiver: Option<String>,
    stmt_end: usize,
}

fn line_start(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let mut at = 0;
    for _ in 1..line {
        at += text[at..].find('\n')? + 1;
    }
    Some(at)
}

fn find_call(text: &str, line: usize, method: &str) -> Option<CallAt> {
    let ls = line_start(text, line)?;
    let le = text[ls..].find('\n').map(|n| ls + n).unwrap_or(text.len());
    let pat_colon = format!(":{method}(");
    let pat_dot = format!(".{method}(");
    let rel = text[ls..le].find(&pat_colon).or_else(|| text[ls..le].find(&pat_dot))?;
    let sep = ls + rel;
    let open = sep + 1 + method.len();
    // matching parenthesis, skipping strings
    let b = text.as_bytes();
    let (mut depth, mut i, mut quote) = (0i32, open, 0u8);
    let close = loop {
        let c = *b.get(i)?;
        if quote != 0 {
            if c == b'\\' {
                i += 1;
            } else if c == quote {
                quote = 0;
            }
        } else {
            match c {
                b'"' | b'\'' => quote = c,
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        break i;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    };
    let args = split_args(&text[open + 1..close]);
    // receiver: identifiers, dots and simple index brackets before the separator
    let mut r = sep;
    let mut bracket = 0;
    while r > ls {
        let c = b[r - 1];
        if c == b']' {
            bracket += 1;
        } else if c == b'[' {
            bracket -= 1;
        } else if bracket == 0 && !(c.is_ascii_alphanumeric() || c == b'_' || c == b'.') {
            break;
        }
        r -= 1;
    }
    let recv = text[r..sep].trim();
    let receiver = (!recv.is_empty() && bracket == 0 && !recv.starts_with('.')).then(|| recv.to_string());
    let stmt_end = text[close..].find('\n').map(|n| close + n).unwrap_or(text.len());
    Some(CallAt { args_start: open + 1, args_end: close, args, receiver, stmt_end })
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut cur) = (0i32, '\0', String::new());
    let mut esc = false;
    for c in s.chars() {
        if quote != '\0' {
            cur.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == quote {
                quote = '\0';
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = c;
                cur.push(c);
            }
            '(' | '{' | '[' => {
                depth += 1;
                cur.push(c);
            }
            ')' | '}' | ']' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur).trim().to_string()),
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn indent_of(text: &str, line: usize) -> String {
    let ls = line_start(text, line).unwrap_or(0);
    text[ls..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// A change as applied: the new text, the 1-based line it touched and a one-line description.
#[derive(Debug, Clone)]
pub struct Patched {
    pub text: String,
    pub line: usize,
    pub what: String,
}

/// Apply `edit` at the recorded call (`site_line`, `site_method`), or, without one, after the creation call
/// (`created_line`, `created_method`).
pub fn apply(
    text: &str,
    site: Option<(usize, &str)>,
    created: Option<(usize, &str)>,
    edit: &Edit,
) -> Result<Patched, String> {
    let method = edit.method();
    if let Some((line, site_method)) = site {
        let call = find_call(text, line, site_method).ok_or_else(|| format!("line {line} has no :{site_method}( call"))?;
        let in_place = site_method == method && edit.literal(&call.args);
        let args = edit.args(&call.args).ok_or("no arguments")?;
        if in_place {
            let mut out = String::with_capacity(text.len() + 16);
            out.push_str(&text[..call.args_start]);
            out.push_str(&args);
            out.push_str(&text[call.args_end..]);
            return Ok(Patched { text: out, line, what: format!("line {line}: {method}({args})") });
        }
        let recv = call.receiver.ok_or_else(|| format!("line {line}: the receiver of :{site_method} is not a plain expression"))?;
        // (a computed anchor target is kept as written: the new call sits right after the old one)
        return Ok(insert_after(text, call.stmt_end, line, &recv, method, &args));
    }
    let (line, cmethod) = created.ok_or("no recorded call and no creation site")?;
    let call = find_call(text, line, cmethod).ok_or_else(|| format!("line {line} has no {cmethod}( call"))?;
    let ls = line_start(text, line).unwrap_or(0);
    let lhs = text[ls..call.args_start].split('=').next().filter(|_| text[ls..call.args_start].contains('='));
    let recv = match lhs {
        Some(l) => l.trim().trim_start_matches("local ").trim().to_string(),
        None => {
            // reflected child: parent:CreateChildWidget(type, "name", 0, true) -> parent.name
            let parent = call.receiver.clone().ok_or("the creation line has no receiver")?;
            let name = call.args.get(1).map(|a| a.trim_matches(|c| c == '"' || c == '\'').to_string()).ok_or("no child name")?;
            let idx = call.args.get(2).map(|s| s.trim().to_string()).unwrap_or_default();
            if idx.is_empty() || idx == "0" { format!("{parent}.{name}") } else { format!("{parent}.{name}[{idx}]") }
        }
    };
    if recv.contains(',') || recv.is_empty() {
        return Err(format!("line {line}: cannot tell the created object's variable"));
    }
    let args = edit.args(&[String::new(), "nil".into()]).ok_or("no arguments")?;
    if matches!(edit, Edit::Anchor { .. }) {
        return Err("this anchor was never set by a script line; set it from the anchor's own call".into());
    }
    Ok(insert_after(text, call.stmt_end, line, &recv, method, &args))
}

fn insert_after(text: &str, stmt_end: usize, line: usize, recv: &str, method: &str, args: &str) -> Patched {
    let ind = indent_of(text, line);
    let new_line = format!("{ind}{recv}:{method}({args})");
    let mut out = String::with_capacity(text.len() + new_line.len() + 1);
    out.push_str(&text[..stmt_end]);
    out.push('\n');
    out.push_str(&new_line);
    out.push_str(&text[stmt_end..]);
    let at = text[..stmt_end].matches('\n').count() + 2;
    Patched { text: out, line: at, what: format!("new line {at}: {}", new_line.trim()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_literal_arguments() {
        let src = "local w = p:CreateChildWidget(\"button\", \"ok\", 0, true)\nw:SetExtent(80, 24)\nw:AddAnchor(\"TOPLEFT\", p, \"TOPLEFT\", 10, 20)\n";
        let r = apply(src, Some((2, "SetExtent")), None, &Edit::Extent(100.0, 30.5)).unwrap();
        assert!(r.text.contains("w:SetExtent(100, 30.5)\n"), "{}", r.text);
        let r = apply(src, Some((3, "AddAnchor")), None, &Edit::Anchor { point: "TOPLEFT".into(), rel: "BOTTOMLEFT".into(), x: 4.0, y: -2.0 }).unwrap();
        assert!(r.text.contains("w:AddAnchor(\"TOPLEFT\", p, \"BOTTOMLEFT\", 4, -2)"), "{}", r.text);
    }

    #[test]
    fn keeps_the_anchor_form() {
        let mv = |p: &str, r: &str, x: f32, y: f32| Edit::Anchor { point: p.into(), rel: r.into(), x, y };
        let src = "newbag:AddAnchor(\"RIGHT\", -30, 0)\nw:AddAnchor(\"TOP\", \"BOTTOM\", 0, 4)\nw:AddAnchor(\"LEFT\", parent)\nw:AddAnchor(\"TOP\", p, 0, ofs)\n";
        let r = apply(src, Some((1, "AddAnchor")), None, &mv("RIGHT", "RIGHT", -330.0, 0.0)).unwrap();
        assert!(r.text.starts_with("newbag:AddAnchor(\"RIGHT\", -330, 0)\n"), "{}", r.text);
        let r = apply(src, Some((2, "AddAnchor")), None, &mv("TOP", "BOTTOM", 2.0, 8.0)).unwrap();
        assert!(r.text.contains("\nw:AddAnchor(\"TOP\", \"BOTTOM\", 2, 8)\n"), "{}", r.text);
        let r = apply(src, Some((3, "AddAnchor")), None, &mv("LEFT", "LEFT", 5.0, 0.0)).unwrap();
        assert!(r.text.contains("\nw:AddAnchor(\"LEFT\", parent, 5, 0)\n"), "{}", r.text);
        // a computed offset: a new call after it, with the target kept
        let r = apply(src, Some((4, "AddAnchor")), None, &mv("TOP", "TOP", 0.0, 12.0)).unwrap();
        assert!(r.text.ends_with("w:AddAnchor(\"TOP\", p, 0, ofs)\nw:AddAnchor(\"TOP\", p, 0, 12)\n"), "{}", r.text);
    }

    #[test]
    fn inserts_after_a_computed_call() {
        let src = "  slot[i]:SetExtent(size,\n    size * 2)\nnext()\n";
        let r = apply(src, Some((1, "SetExtent")), None, &Edit::Extent(40.0, 40.0)).unwrap();
        assert_eq!(r.text, "  slot[i]:SetExtent(size,\n    size * 2)\n  slot[i]:SetExtent(40, 40)\nnext()\n");
        assert_eq!(r.line, 3);
    }

    #[test]
    fn falls_back_to_the_creation_line() {
        let src = "wnd:CreateChildWidget(\"button\", \"closeButton\", 0, true)\nlocal t = wnd:CreateChildWidget('label', 'title', 0, true)\n";
        let r = apply(src, None, Some((1, "CreateChildWidget")), &Edit::Show(false)).unwrap();
        assert!(r.text.contains("\nwnd.closeButton:Show(false)\n"), "{}", r.text);
        let r = apply(src, None, Some((2, "CreateChildWidget")), &Edit::Text("Hi \"you\"".into())).unwrap();
        assert!(r.text.ends_with("t:SetText(\"Hi \\\"you\\\"\")\n"), "{}", r.text);
    }

    #[test]
    fn numbers() {
        assert_eq!(num(3.0), "3");
        assert_eq!(num(-0.25), "-0.25");
        assert_eq!(num(1.0 / 3.0), "0.333");
    }
}
