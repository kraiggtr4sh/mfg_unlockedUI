//! Minimal parser for Valve's KeyValues (VDF/ACF) text format.

#[derive(Debug, Clone)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// Case-insensitive child lookup.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(items) => items.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Vdf::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn children(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(items) => items,
            _ => &[],
        }
    }
}

enum Tok {
    S(String),
    Open,
    Close,
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '"' => {
                let mut s = String::new();
                while let Some(ch) = it.next() {
                    match ch {
                        '\\' => {
                            if let Some(n) = it.next() {
                                match n {
                                    'n' => s.push('\n'),
                                    't' => s.push('\t'),
                                    other => s.push(other),
                                }
                            }
                        }
                        '"' => break,
                        other => s.push(other),
                    }
                }
                out.push(Tok::S(s));
            }
            '/' if it.peek() == Some(&'/') => {
                for ch in it.by_ref() {
                    if ch == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            other => {
                // Unquoted token
                let mut s = String::from(other);
                while let Some(&ch) = it.peek() {
                    if ch.is_whitespace() || ch == '{' || ch == '}' || ch == '"' {
                        break;
                    }
                    s.push(ch);
                    it.next();
                }
                out.push(Tok::S(s));
            }
        }
    }
    out
}

fn parse_obj(toks: &[Tok], pos: &mut usize) -> Vdf {
    let mut items = Vec::new();
    while *pos < toks.len() {
        match &toks[*pos] {
            Tok::Close => {
                *pos += 1;
                break;
            }
            Tok::S(key) => {
                let key = key.clone();
                *pos += 1;
                if *pos >= toks.len() {
                    break;
                }
                match &toks[*pos] {
                    Tok::S(v) => {
                        items.push((key, Vdf::Str(v.clone())));
                        *pos += 1;
                    }
                    Tok::Open => {
                        *pos += 1;
                        items.push((key, parse_obj(toks, pos)));
                    }
                    Tok::Close => {}
                }
            }
            Tok::Open => {
                *pos += 1;
            }
        }
    }
    Vdf::Obj(items)
}

pub fn parse(src: &str) -> Vdf {
    let toks = tokenize(src);
    let mut pos = 0;
    parse_obj(&toks, &mut pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_folders() {
        let src = r#"
"libraryfolders"
{
    "0"
    {
        "path"		"C:\\Program Files (x86)\\Steam"
        "apps" { "228980" "1234" }
    }
    "1" { "path" "D:\\SteamLibrary" }
}"#;
        let v = parse(src);
        let lf = v.get("libraryfolders").unwrap();
        let paths: Vec<_> = lf.children().iter().filter_map(|(_, o)| o.str("path")).collect();
        assert_eq!(paths, vec!["C:\\Program Files (x86)\\Steam", "D:\\SteamLibrary"]);
    }
}
