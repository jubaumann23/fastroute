//! Generic S-expression reader for Specctra DSN / SES files.
//!
//! The Java implementation uses a context-sensitive JFlex scanner that decides
//! how to tokenize based on the preceding keyword. Here the tokenizer is
//! context-free (atoms, quoted strings, brackets) and all interpretation
//! (numbers, `component-pin` splitting, keywords) happens in the typed layer.
//!
//! Quirks handled:
//! * `(string_quote ")` — the token after `string_quote` is read raw, so the
//!   quote character itself does not open a string.
//! * A quote followed by a delimiter (whitespace, bracket, EOF) closes the
//!   string. A quote followed by another quote is a literal quote character:
//!   KiCad writes values like `"0.5""` for a literal inch mark, which a
//!   first-quote-closes rule would desynchronize.
//! * A quote followed by any other character closes the quoted part and the
//!   rest is read as a bare suffix of the same atom: `"CR2032-3V"-2` becomes
//!   `CR2032-3V-2` with `quoted_len = Some(9)`, preserving the component/pin
//!   boundary for pin references.
//! * Input need not be valid UTF-8; invalid sequences are replaced.

use std::fmt;

#[derive(Clone, PartialEq)]
pub enum Sexpr {
    Atom(Atom),
    List(List),
}

#[derive(Clone, PartialEq)]
pub struct Atom {
    pub text: String,
    pub quoted: bool,
    /// For `"quoted"suffix` atoms: byte length of the quoted prefix in `text`.
    pub quoted_len: Option<usize>,
    pub line: u32,
}

#[derive(Clone, PartialEq)]
pub struct List {
    pub items: Vec<Sexpr>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: u32,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

impl Sexpr {
    pub fn as_atom(&self) -> Option<&Atom> {
        match self {
            Sexpr::Atom(a) => Some(a),
            Sexpr::List(_) => None,
        }
    }

    pub fn as_list(&self) -> Option<&List> {
        match self {
            Sexpr::List(l) => Some(l),
            Sexpr::Atom(_) => None,
        }
    }

    pub fn line(&self) -> u32 {
        match self {
            Sexpr::Atom(a) => a.line,
            Sexpr::List(l) => l.line,
        }
    }
}

impl List {
    /// The leading keyword of the list, lower-cased comparison is up to the caller.
    pub fn head(&self) -> Option<&str> {
        match self.items.first() {
            Some(Sexpr::Atom(a)) if !a.quoted => Some(&a.text),
            _ => None,
        }
    }

    /// True if the head keyword equals `kw` (ASCII case-insensitive, as in Specctra).
    pub fn is(&self, kw: &str) -> bool {
        self.head().is_some_and(|h| h.eq_ignore_ascii_case(kw))
    }

    /// Items after the head keyword.
    pub fn args(&self) -> &[Sexpr] {
        if self.items.is_empty() {
            &[]
        } else {
            &self.items[1..]
        }
    }

    /// Child lists (skipping atoms) after the head.
    pub fn sublists(&self) -> impl Iterator<Item = &List> {
        self.args().iter().filter_map(Sexpr::as_list)
    }

    /// First child list with the given head keyword.
    pub fn find(&self, kw: &str) -> Option<&List> {
        self.sublists().find(|l| l.is(kw))
    }

    /// All child lists with the given head keyword.
    pub fn find_all<'a>(&'a self, kw: &'a str) -> impl Iterator<Item = &'a List> + 'a {
        self.sublists().filter(move |l| l.is(kw))
    }

    /// Leading atoms after the head (stops at the first sub-list).
    pub fn leading_atoms(&self) -> impl Iterator<Item = &Atom> {
        self.args().iter().map_while(Sexpr::as_atom)
    }
}

impl fmt::Debug for Sexpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Sexpr::Atom(a) => a.fmt(f),
            Sexpr::List(l) => l.fmt(f),
        }
    }
}

impl fmt::Debug for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.quoted {
            write!(f, "{:?}", self.text)
        } else {
            f.write_str(&self.text)
        }
    }
}

impl fmt::Debug for List {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            item.fmt(f)?;
        }
        f.write_str(")")
    }
}

fn is_delim(b: u8) -> bool {
    b.is_ascii_whitespace() || b == b'(' || b == b')'
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    quote: u8,
}

enum Token {
    Open(u32),
    Close(u32),
    Atom(Atom),
}

impl<'a> Lexer<'a> {
    fn skip_ws(&mut self) {
        while let Some(&b) = self.src.get(self.pos) {
            if b == b'\n' {
                self.line += 1;
            } else if !b.is_ascii_whitespace() {
                break;
            }
            self.pos += 1;
        }
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// Advances over a run of non-delimiter bytes.
    fn skip_bare(&mut self) {
        while self.src.get(self.pos).is_some_and(|&b| !is_delim(b)) {
            self.pos += 1;
        }
    }

    /// Reads a bare atom; quote characters inside it are ordinary characters.
    fn bare(&mut self) -> Atom {
        let start = self.pos;
        self.skip_bare();
        Atom {
            text: Self::text(&self.src[start..self.pos]),
            quoted: false,
            quoted_len: None,
            line: self.line,
        }
    }

    fn quoted(&mut self, q: u8) -> Result<Atom, ParseError> {
        let line = self.line;
        self.pos += 1; // opening quote
        let mut buf = Vec::new();
        loop {
            let Some(&b) = self.src.get(self.pos) else {
                return Err(ParseError {
                    line,
                    message: "unterminated quoted string".into(),
                });
            };
            self.pos += 1;
            if b != q {
                if b == b'\n' {
                    self.line += 1;
                }
                buf.push(b);
                continue;
            }
            match self.src.get(self.pos) {
                Some(&n) if n == q => buf.push(q),
                Some(&n) if !is_delim(n) => {
                    // `"quoted"suffix`; the suffix may itself contain quoted
                    // segments, e.g. `"J3 -+"-"_VBUS #22"`.
                    let quoted_len = buf.len();
                    while let Some(&c) = self.src.get(self.pos) {
                        if is_delim(c) {
                            break;
                        }
                        self.pos += 1;
                        if c != q {
                            buf.push(c);
                            continue;
                        }
                        while let Some(&c) = self.src.get(self.pos) {
                            self.pos += 1;
                            if c == q {
                                break;
                            }
                            if c == b'\n' {
                                self.line += 1;
                            }
                            buf.push(c);
                        }
                    }
                    let text = Self::text(&buf);
                    // Lossy conversion may change byte lengths; re-derive safely.
                    let quoted_len = text.is_char_boundary(quoted_len).then_some(quoted_len);
                    return Ok(Atom {
                        text,
                        quoted: true,
                        quoted_len,
                        line,
                    });
                }
                _ => {
                    return Ok(Atom {
                        text: Self::text(&buf),
                        quoted: true,
                        quoted_len: None,
                        line,
                    })
                }
            }
        }
    }

    fn next(&mut self, raw: bool) -> Result<Option<Token>, ParseError> {
        self.skip_ws();
        let Some(&b) = self.src.get(self.pos) else {
            return Ok(None);
        };
        let tok = match b {
            b'(' => {
                self.pos += 1;
                Token::Open(self.line)
            }
            b')' => {
                self.pos += 1;
                Token::Close(self.line)
            }
            _ if !raw && (b == self.quote || b == b'"') => Token::Atom(self.quoted(b)?),
            _ => Token::Atom(self.bare()),
        };
        Ok(Some(tok))
    }
}

/// Parses a complete document; returns all top-level expressions.
pub fn parse(src: &[u8]) -> Result<Vec<Sexpr>, ParseError> {
    let mut lx = Lexer {
        src,
        pos: 0,
        line: 1,
        quote: b'"',
    };
    let mut stack: Vec<List> = Vec::new();
    let mut top = Vec::new();
    // Set when the previous token was the `string_quote` head keyword.
    let mut raw_next = false;
    while let Some(tok) = lx.next(raw_next)? {
        let was_raw = std::mem::take(&mut raw_next);
        match tok {
            Token::Open(line) => stack.push(List {
                items: Vec::new(),
                line,
            }),
            Token::Close(line) => {
                let list = stack.pop().ok_or(ParseError {
                    line,
                    message: "unbalanced ')'".into(),
                })?;
                match stack.last_mut() {
                    Some(parent) => parent.items.push(Sexpr::List(list)),
                    None => top.push(Sexpr::List(list)),
                }
            }
            Token::Atom(atom) => {
                let Some(cur) = stack.last_mut() else {
                    return Err(ParseError {
                        line: atom.line,
                        message: format!("atom {:?} outside of any list", atom.text),
                    });
                };
                if was_raw {
                    if let Some(&q) = atom.text.as_bytes().first() {
                        lx.quote = q;
                    }
                } else if cur.items.is_empty()
                    && !atom.quoted
                    && atom.text.eq_ignore_ascii_case("string_quote")
                {
                    raw_next = true;
                }
                cur.items.push(Sexpr::Atom(atom));
            }
        }
    }
    // Tolerate missing closing brackets at EOF (truncated files): close
    // everything that is still open.
    while let Some(list) = stack.pop() {
            match stack.last_mut() {
                Some(parent) => parent.items.push(Sexpr::List(list)),
            None => top.push(Sexpr::List(list)),
        }
    }
    Ok(top)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(src: &str) -> List {
        let v = parse(src.as_bytes()).unwrap();
        assert_eq!(v.len(), 1);
        v[0].as_list().unwrap().clone()
    }

    #[test]
    fn basic() {
        let l = one("(pcb x (a 1 2.5) (b \"q s\"))");
        assert!(l.is("PCB"));
        assert_eq!(format!("{l:?}"), "(pcb x (a 1 2.5) (b \"q s\"))");
    }

    #[test]
    fn string_quote_and_inch_mark() {
        let l = one("(pcb (parser (string_quote \")) (PN \"DISPLAY 0.5\"\"))");
        let pn = l.find("PN").unwrap();
        assert_eq!(pn.args()[0].as_atom().unwrap().text, "DISPLAY 0.5\"");
        let p = l.find("parser").unwrap().find("string_quote").unwrap();
        assert_eq!(p.args()[0].as_atom().unwrap().text, "\"");
    }

    #[test]
    fn quoted_component_with_pin_suffix() {
        let l = one("(pins \"CR2032-3V\"-2 J2-2)");
        let a = l.args()[0].as_atom().unwrap();
        assert_eq!(a.text, "CR2032-3V-2");
        assert_eq!(a.quoted_len, Some(9));
        assert_eq!(l.args()[1].as_atom().unwrap().text, "J2-2");

        let l = one("(pins \"J3 -+\"-\"_VBUS #22\" X-1)");
        let a = l.args()[0].as_atom().unwrap();
        assert_eq!(a.text, "J3 -+-_VBUS #22");
        assert_eq!(a.quoted_len, Some(5));
        assert_eq!(l.args().len(), 2);
    }

    #[test]
    fn apostrophe_inside_atom() {
        let l = one("(host_cad KiCad's)");
        assert_eq!(l.args()[0].as_atom().unwrap().text, "KiCad's");
    }

    #[test]
    fn empty_string() {
        let l = one("(keepout \"\" (polygon signal 0))");
        let a = l.args()[0].as_atom().unwrap();
        assert!(a.quoted && a.text.is_empty());
    }
}
