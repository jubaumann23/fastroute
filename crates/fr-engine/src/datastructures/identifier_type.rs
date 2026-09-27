//! Port of `datastructures/IdentifierType.java`: writes identifiers (component, net, padstack,
//! layer names ...) to Specctra files, quoting them when necessary.
//!
//! Byte-exact with Java, including its quirks. The processing works on UTF-16 code units like
//! Java strings do:
//!
//! * While the name is longer than 2 units and starts and ends with `"`, it is replaced by
//!   `substring(1, length - 2)` -- this drops the closing quote *and the unit before it* (a
//!   Java bug, reproduced). Cutting a surrogate pair leaves an unpaired high surrogate at the
//!   end; Java's `OutputStreamWriter` keeps it pending and writes `?` only when the next write
//!   does not start with a low surrogate ([`IndentFileWriter::write_utf16`] emulates this).
//! * All occurrences of the quote string are removed.
//! * Quotes are added if the name contains a reserved string, a NUL or non-ASCII character
//!   (UTF-8 byte `<= 0`; an unpaired surrogate encodes as `?` and does not count), or matches
//!   `^-?\d.*` (an ASCII digit, optionally after `-`, and no line terminator
//!   `U+000A U+000D U+0085 U+2028 U+2029` after it).

use std::io;

use super::IndentFileWriter;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentifierType {
    string_quote: Vec<u16>,
    reserved_chars: Vec<Vec<u16>>,
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Java `String.contains` on UTF-16 units (`contains("")` is true).
fn contains(haystack: &[u16], needle: &[u16]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|w| w == needle)
}

/// Java `String.replace(target, "")` on UTF-16 units (non-overlapping, left to right; an empty
/// target leaves the string unchanged because the replacement is empty).
fn remove_all(s: &[u16], target: &[u16]) -> Vec<u16> {
    if target.is_empty() {
        return s.to_vec();
    }
    let mut result = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.len() - i >= target.len() && &s[i..i + target.len()] == target {
            i += target.len();
        } else {
            result.push(s[i]);
            i += 1;
        }
    }
    result
}

/// Java `name.matches("^-?\\d.*")`.
fn starts_like_number(s: &[u16]) -> bool {
    let mut i = 0;
    if s.first() == Some(&(b'-' as u16)) {
        i = 1;
    }
    match s.get(i) {
        Some(&c) if (b'0' as u16..=b'9' as u16).contains(&c) => {}
        _ => return false,
    }
    // `.` matches everything except line terminators.
    !s[i + 1..].iter().any(|&c| matches!(c, 0x0A | 0x0D | 0x85 | 0x2028 | 0x2029))
}

/// UTF-8 encoding of the name contains a byte `<= 0` (as a signed Java byte).
fn has_nul_or_non_ascii(s: &[u16]) -> bool {
    char::decode_utf16(s.iter().copied()).any(|c| match c {
        Ok(c) => c == '\0' || !c.is_ascii(),
        Err(_) => false, // encoded as '?'
    })
}

impl IdentifierType {
    /// Java `new IdentifierType(reservedChars, stringQuote)`.
    pub fn new<S: AsRef<str>>(reserved_chars: &[S], string_quote: &str) -> Self {
        IdentifierType {
            string_quote: utf16(string_quote),
            reserved_chars: reserved_chars.iter().map(|s| utf16(s.as_ref())).collect(),
        }
    }

    /// The Java string `write(name, file)` passes to the writer, as UTF-16 code units (it may
    /// contain unpaired surrogates, see the module documentation).
    pub fn java_string(&self, name: &[u16]) -> Vec<u16> {
        let quote = &self.string_quote;
        let mut name = name.to_vec();
        let dq = b'"' as u16;
        // remove the double quotes from the identifiers
        while name.len() > 2 && name[0] == dq && name[name.len() - 1] == dq {
            name = name[1..name.len() - 2].to_vec();
        }

        // if the name contains our quote character, we must remove it
        if contains(&name, quote) {
            name = remove_all(&name, quote);
        }

        // if the name contains a reserved character, we must put it into quotes
        let mut need_quotes = self.reserved_chars.iter().any(|r| contains(&name, r));

        // if the name contains a non-ASCII character, we must put it into quotes
        if !need_quotes && has_nul_or_non_ascii(&name) {
            need_quotes = true;
        }

        if !need_quotes && starts_like_number(&name) {
            need_quotes = true;
        }
        if need_quotes {
            let mut quoted = Vec::with_capacity(name.len() + 2 * quote.len());
            quoted.extend_from_slice(quote);
            quoted.extend_from_slice(&name);
            quoted.extend_from_slice(quote);
            name = quoted;
        }
        name
    }

    /// The written text as a Rust string, unpaired surrogates replaced by `?` (which is what
    /// Java writes when more text follows; see [`IndentFileWriter::write_utf16`]).
    pub fn format(&self, name: &str) -> String {
        char::decode_utf16(self.java_string(&utf16(name))).map(|c| c.unwrap_or('?')).collect()
    }

    /// Java `write(name, file)`. (Java logs and swallows an `IOException`; here it is
    /// returned.)
    pub fn write<W: io::Write>(&self, name: &str, file: &mut IndentFileWriter<W>) -> io::Result<()> {
        file.write_utf16(&self.java_string(&utf16(name)))
    }

    /// [`write`](Self::write) for a name given as UTF-16 code units.
    pub fn write_utf16<W: io::Write>(&self, name: &[u16], file: &mut IndentFileWriter<W>) -> io::Result<()> {
        file.write_utf16(&self.java_string(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `IdentifierTypeTest.write`.
    #[test]
    fn java_identifier_type_test() {
        let it = IdentifierType::new(&["(", ")", " ", "-"], "\"");
        assert_eq!(it.format("600"), "\"600\"");
        assert_eq!(it.format("-600"), "\"-600\"");
        assert_eq!(it.format("test"), "test");
        assert_eq!(it.format("test-with-reserved"), "\"test-with-reserved\"");
        assert_eq!(it.format("600a"), "\"600a\"");
    }

    #[test]
    fn quirks() {
        let ses = IdentifierType::new(&["(", ")", " ", ";", "-", "_", "/", "~", "{", "}"], "\"");
        // substring(1, len - 2) drops the character before the closing quote
        assert_eq!(ses.format("\"abc\""), "ab");
        assert_eq!(ses.format("\"\"x\"\""), "x"); // "x" after one cut is `"x`, then quote removed
        assert_eq!(ses.format("\"\""), "");
        assert_eq!(ses.format("a\"b"), "ab");
        assert_eq!(ses.format("R1"), "R1");
        assert_eq!(ses.format("-"), "\"-\"");
        assert_eq!(ses.format("-a"), "\"-a\"");
        assert_eq!(ses.format("1\n"), "1\n");
        assert_eq!(ses.format("1a\u{2028}"), "\"1a\u{2028}\""); // non-ASCII
        assert_eq!(ses.format("\u{e9}"), "\"\u{e9}\"");
        assert_eq!(ses.format("a\0"), "\"a\0\"");
        // cut surrogate pair -> '?' when more text follows, nothing when the writer is only
        // flushed (Java's encoder keeps a trailing high surrogate pending)
        assert_eq!(ses.format("\"a\u{1F600}\""), "a?");
        let mut w = IndentFileWriter::new(Vec::new());
        ses.write("\"a\u{1F600}\"", &mut w).unwrap();
        assert_eq!(w.get_ref().as_slice(), b"a");
        w.write(")").unwrap();
        assert_eq!(w.into_inner().unwrap(), b"a?)");
        let empty_quote = IdentifierType::new(&["("], "");
        assert_eq!(empty_quote.format("1a"), "1a");
        assert_eq!(empty_quote.format("x(y"), "x(y");
    }

    fn unhex16(s: &str) -> Vec<u16> {
        if s == "-" {
            return Vec::new();
        }
        (0..s.len() / 4).map(|i| u16::from_str_radix(&s[4 * i..4 * i + 4], 16).unwrap()).collect()
    }

    fn unhex8(s: &str) -> Vec<u8> {
        if s == "-" {
            return Vec::new();
        }
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// Byte-exact comparison with the JDK (`testdata/DatastructuresGen.java identifier`).
    /// Inputs containing unpaired surrogates cannot be `&str`; they are exercised through
    /// the UTF-16 entry point.
    #[test]
    fn jdk_ground_truth() {
        let data = include_str!("testdata/identifier.txt");
        let mut n = 0;
        for line in data.lines() {
            let t: Vec<&str> = line.split(' ').collect();
            assert_eq!(t[0], "S");
            let quote = String::from_utf16(&unhex16(t[1])).unwrap();
            let reserved: Vec<String> =
                t[2].split(',').map(|h| String::from_utf16(&unhex16(h)).unwrap()).collect();
            let it = IdentifierType::new(&reserved, &quote);
            let input = unhex16(t[3]);
            let expected = unhex8(t[4]);
            // Java writes each name to a fresh OutputStreamWriter and flushes it.
            let mut w = IndentFileWriter::new(Vec::new());
            it.write_utf16(&input, &mut w).unwrap();
            assert_eq!(w.into_inner().unwrap(), expected, "{line}");
            if let Ok(s) = String::from_utf16(&input) {
                let mut w = IndentFileWriter::new(Vec::new());
                it.write(&s, &mut w).unwrap();
                assert_eq!(w.into_inner().unwrap(), expected, "{line}");
            }
            n += 1;
        }
        assert!(n > 5000);
    }
}
