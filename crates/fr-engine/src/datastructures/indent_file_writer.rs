//! Port of `datastructures/IndentFileWriter.java`: an UTF-8 text writer that indents scopes
//! (used by the SES, DSN and rules writers).
//!
//! Output format: `start_scope(true)` writes `"\n"` + two spaces per open scope + `"("` and opens
//! a scope; `start_scope(false)` omits the line break; `end_scope()` closes the scope first and
//! then writes `"\n"` + indentation + `")"`; `new_line()` writes `"\n"` + indentation. No `\r`.
//!
//! Encoding: Java's `OutputStreamWriter` (UTF-8) keeps a high surrogate at the end of a write
//! pending until the next write: it is combined with a following low surrogate, replaced by
//! `?` otherwise, and dropped by `flush()` (written as `?` only by `close()`). Unpaired
//! surrogates elsewhere become `?`. [`write_utf16`](IndentFileWriter::write_utf16) reproduces
//! this for Java strings given as UTF-16; [`into_inner`](IndentFileWriter::into_inner)
//! corresponds to `flush()` without `close()` (what the SES writer does),
//! [`close`](IndentFileWriter::close) to `close()`.
//!
//! Errors: Java's `write(String)` throws `IOException`, while `startScope`, `endScope` and
//! `newLine` log it and continue. Here [`write`](IndentFileWriter::write) returns the error and
//! the scope methods log it; the first error is also remembered ([`error`](IndentFileWriter::error)).

use std::fmt;
use std::io::{self, Write};

const INDENT_STRING: &str = "  ";
const BEGIN_SCOPE: &str = "(";
const END_SCOPE: &str = ")";

#[derive(Debug)]
pub struct IndentFileWriter<W: Write> {
    out: W,
    current_indent_level: i32,
    error: Option<io::ErrorKind>,
    /// High surrogate at the end of the last write (Java `StreamEncoder.leftoverChar`).
    pending_high_surrogate: Option<u16>,
}

#[inline]
fn is_high(u: u16) -> bool {
    (0xD800..0xDC00).contains(&u)
}

#[inline]
fn is_low(u: u16) -> bool {
    (0xDC00..0xE000).contains(&u)
}

impl<W: Write> IndentFileWriter<W> {
    /// Java `new IndentFileWriter(stream)`.
    pub fn new(out: W) -> Self {
        IndentFileWriter { out, current_indent_level: 0, error: None, pending_high_surrogate: None }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        let r = self.out.write_all(bytes);
        if let Err(e) = &r {
            self.error.get_or_insert(e.kind());
        }
        r
    }

    /// Java `write(String)`.
    pub fn write(&mut self, s: &str) -> io::Result<()> {
        if self.pending_high_surrogate.is_some() && !s.is_empty() {
            // a Rust string never starts with a low surrogate
            self.pending_high_surrogate = None;
            self.write_bytes(b"?")?;
        }
        self.write_bytes(s.as_bytes())
    }

    /// Java `write(String)` for a Java string given as UTF-16 code units, which may contain
    /// unpaired surrogates (encoded like Java's `StreamEncoder`, see the module documentation).
    pub fn write_utf16(&mut self, units: &[u16]) -> io::Result<()> {
        let mut buf = String::with_capacity(units.len());
        let mut i = 0;
        if let Some(high) = self.pending_high_surrogate {
            if units.is_empty() {
                return Ok(());
            }
            self.pending_high_surrogate = None;
            if is_low(units[0]) {
                buf.push(char::decode_utf16([high, units[0]]).next().unwrap().unwrap());
                i = 1;
            } else {
                buf.push('?');
            }
        }
        while i < units.len() {
            let u = units[i];
            if is_high(u) {
                if i + 1 == units.len() {
                    self.pending_high_surrogate = Some(u);
                    i += 1;
                    continue;
                }
                if is_low(units[i + 1]) {
                    buf.push(char::decode_utf16([u, units[i + 1]]).next().unwrap().unwrap());
                    i += 2;
                    continue;
                }
                buf.push('?');
            } else if is_low(u) {
                buf.push('?');
            } else {
                buf.push(char::from_u32(u as u32).unwrap());
            }
            i += 1;
        }
        self.write_bytes(buf.as_bytes())
    }

    /// Java `startScope(newLine)`: begins a new scope.
    pub fn start_scope(&mut self, new_line: bool) {
        if new_line {
            self.new_line();
        }
        if self.write(BEGIN_SCOPE).is_err() {
            log::error!("IndentFileWriter.start_scope: unable to write to file");
        }
        self.current_indent_level += 1;
    }

    /// Java `startScope()`: begins a new scope on a new line.
    pub fn start_scope_new_line(&mut self) {
        self.start_scope(true);
    }

    /// Java `endScope()`: closes the latest open scope.
    pub fn end_scope(&mut self) {
        self.current_indent_level -= 1;
        self.new_line();
        if self.write(END_SCOPE).is_err() {
            log::error!("IndentFileWriter.end_scope: unable to write to file");
        }
    }

    /// Java `newLine()`: starts a new line inside a scope.
    pub fn new_line(&mut self) {
        let mut ok = self.write("\n").is_ok();
        if ok {
            for _ in 0..self.current_indent_level {
                if self.write(INDENT_STRING).is_err() {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            log::error!("IndentFileWriter.newLine: unable to write to file");
        }
    }

    /// The current number of open scopes.
    pub fn indent_level(&self) -> i32 {
        self.current_indent_level
    }

    /// The kind of the first I/O error that occurred, if any.
    pub fn error(&self) -> Option<io::ErrorKind> {
        self.error
    }

    /// Java `flush()`.
    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    pub fn get_ref(&self) -> &W {
        &self.out
    }

    /// Java `flush()` and returns the underlying writer; a pending high surrogate is dropped.
    pub fn into_inner(mut self) -> io::Result<W> {
        self.out.flush()?;
        Ok(self.out)
    }

    /// Java `close()`: writes a pending high surrogate as `?`, flushes and returns the
    /// underlying writer.
    pub fn close(mut self) -> io::Result<W> {
        if self.pending_high_surrogate.take().is_some() {
            self.write_bytes(b"?")?;
        }
        self.into_inner()
    }
}

/// Lets `write!` target the writer.
impl<W: Write> fmt::Write for IndentFileWriter<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        IndentFileWriter::write(self, s).map_err(|_| fmt::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_and_indentation() {
        let mut w = IndentFileWriter::new(Vec::new());
        w.start_scope(false);
        w.write("session a.ses").unwrap();
        w.start_scope_new_line();
        w.write("x").unwrap();
        w.start_scope(true);
        w.new_line();
        w.end_scope();
        w.end_scope();
        w.end_scope();
        w.end_scope(); // negative level: no indentation
        w.new_line();
        let s = String::from_utf8(w.into_inner().unwrap()).unwrap();
        assert_eq!(s, "(session a.ses\n  (x\n    (\n      \n    )\n  )\n)\n)\n");
    }

    #[test]
    fn utf8_output() {
        let mut w = IndentFileWriter::new(Vec::new());
        fmt::Write::write_str(&mut w, "\u{e9}").unwrap();
        assert_eq!(w.get_ref().as_slice(), "\u{e9}".as_bytes());
    }

    /// Java `OutputStreamWriter` surrogate handling (values checked against JDK 25).
    #[test]
    fn surrogates_like_java_stream_encoder() {
        fn run(parts: &[&[u16]], close: bool) -> Vec<u8> {
            let mut w = IndentFileWriter::new(Vec::new());
            for p in parts {
                w.write_utf16(p).unwrap();
            }
            if close {
                w.close().unwrap()
            } else {
                w.into_inner().unwrap()
            }
        }
        let (a, b, x, hi, lo) = (0x61u16, 0x62u16, 0x78u16, 0xD83Du16, 0xDE00u16);
        assert_eq!(run(&[&[a, hi]], false), b"a");
        assert_eq!(run(&[&[a, hi]], true), b"a?");
        assert_eq!(run(&[&[a, hi], &[lo, b]], false), "a\u{1F600}b".as_bytes());
        assert_eq!(run(&[&[a, hi], &[b]], false), b"a?b");
        assert_eq!(run(&[&[a, hi], &[hi, lo]], false), "a?\u{1F600}".as_bytes());
        assert_eq!(run(&[&[lo, x]], false), b"?x");
        assert_eq!(run(&[&[hi, x]], false), b"?x");
        assert_eq!(run(&[&[hi, hi], &[x]], false), b"??x");
        assert_eq!(run(&[&[hi], &[], &[lo]], false), "\u{1F600}".as_bytes());
        let mut w = IndentFileWriter::new(Vec::new());
        w.write_utf16(&[hi]).unwrap();
        w.write("x").unwrap();
        assert_eq!(w.into_inner().unwrap(), b"?x");
    }

    /// Byte-exact comparison with the JDK (`testdata/DatastructuresGen.java indent`).
    #[test]
    fn jdk_ground_truth() {
        let data = include_str!("testdata/indent.txt");
        let unhex = |s: &str, w: usize| -> Vec<u32> {
            if s == "-" {
                return Vec::new();
            }
            (0..s.len() / w).map(|i| u32::from_str_radix(&s[w * i..w * i + w], 16).unwrap()).collect()
        };
        let mut n = 0;
        for line in data.lines() {
            let t: Vec<&str> = line.split(' ').collect();
            let mut w = IndentFileWriter::new(Vec::new());
            for op in t[1].split(',') {
                match op {
                    "s1" => w.start_scope(true),
                    "s0" => w.start_scope(false),
                    "e" => w.end_scope(),
                    "n" => w.new_line(),
                    _ => {
                        let units: Vec<u16> = unhex(&op[1..], 4).into_iter().map(|u| u as u16).collect();
                        w.write_utf16(&units).unwrap();
                    }
                }
            }
            let expected: Vec<u8> = unhex(t[2], 2).into_iter().map(|b| b as u8).collect();
            assert_eq!(w.into_inner().unwrap(), expected, "{line}");
            n += 1;
        }
        assert_eq!(n, 60);
    }
}
