//! Byte-preserving `.ini` reader for TS3 identity files.
//!
//! Only the `identity="…"` value is meaningful to this crate; everything
//! else is held verbatim so that `parse → serialize` roundtrips are
//! byte-identical except for the counter digits we change deliberately.

use crate::error::{Error, Result};

/// A single physical line of the file plus its line terminator.
///
/// We model the identity line specially because its value (the
/// `<counter>V<blob>` payload between the quotes) is the only thing this
/// crate is ever going to mutate.
#[derive(Clone, Debug)]
enum Line {
    Verbatim(Vec<u8>),
    Identity {
        prefix: Vec<u8>, // bytes up to and including the opening `"`
        counter: u64,
        blob_b64: String,
        suffix: Vec<u8>, // closing `"` + rest of the line incl. terminator
    },
}

#[derive(Clone, Debug)]
pub struct IdentityFile {
    lines: Vec<Line>,
    identity_idx: usize,
}

/// Convenience accessor helper for looking up a `key=value` pair on a
/// Verbatim line. We don't model these explicitly because we never need
/// to mutate them — but we do want to display them in the GUI.
fn lookup_kv<'a>(lines: &'a [Line], key: &str) -> Option<&'a str> {
    for line in lines {
        if let Line::Verbatim(bytes) = line {
            let s = std::str::from_utf8(bytes).ok()?;
            // Strip CRLF/LF.
            let s = s.trim_end_matches('\n').trim_end_matches('\r');
            let mut t = s.trim_start_matches([' ', '\t']);
            if let Some(rest) = t.strip_prefix(key) {
                t = rest.trim_start_matches([' ', '\t']);
                if let Some(value) = t.strip_prefix('=') {
                    return Some(value.trim_start_matches([' ', '\t']));
                }
            }
        }
    }
    None
}

impl IdentityFile {
    /// Parse a `.ini` file from its raw bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut lines = Vec::new();
        let mut identity_idx: Option<usize> = None;

        for raw in split_lines(bytes) {
            if identity_idx.is_some() {
                lines.push(Line::Verbatim(raw.to_vec()));
                continue;
            }

            match try_parse_identity_line(raw) {
                Some(parsed) => {
                    identity_idx = Some(lines.len());
                    lines.push(parsed);
                }
                None => lines.push(Line::Verbatim(raw.to_vec())),
            }
        }

        let identity_idx = identity_idx.ok_or(Error::NoIdentityKey)?;
        Ok(Self {
            lines,
            identity_idx,
        })
    }

    /// Decimal counter currently encoded before the `V`.
    pub fn counter(&self) -> u64 {
        match &self.lines[self.identity_idx] {
            Line::Identity { counter, .. } => *counter,
            _ => unreachable!(),
        }
    }

    /// Base64-encoded, obfuscated keypair blob (the part after `V`).
    pub fn blob_b64(&self) -> &str {
        match &self.lines[self.identity_idx] {
            Line::Identity { blob_b64, .. } => blob_b64.as_str(),
            _ => unreachable!(),
        }
    }

    /// Value of the `nickname=` line if present.
    pub fn nickname(&self) -> Option<&str> {
        lookup_kv(&self.lines, "nickname")
    }

    /// Value of the `phonetic_nickname=` line if present.
    pub fn phonetic_nickname(&self) -> Option<&str> {
        lookup_kv(&self.lines, "phonetic_nickname")
    }

    /// Value of the `id=` line if present. This is TS3's local-only
    /// identifier — not derived from the key.
    pub fn local_id(&self) -> Option<&str> {
        lookup_kv(&self.lines, "id")
    }

    /// Replace the counter in the in-memory representation. The base64
    /// blob, ordering, comments, whitespace, and line terminators are
    /// preserved.
    pub fn set_counter(&mut self, new_counter: u64) {
        match &mut self.lines[self.identity_idx] {
            Line::Identity { counter, .. } => *counter = new_counter,
            _ => unreachable!(),
        }
    }

    /// Serialize back to bytes. The output is byte-identical to the input
    /// except that the counter digits inside the `identity="…"` value
    /// reflect the current value of [`set_counter`].
    pub fn serialize(&self, out: &mut Vec<u8>) {
        for line in &self.lines {
            match line {
                Line::Verbatim(bytes) => out.extend_from_slice(bytes),
                Line::Identity {
                    prefix,
                    counter,
                    blob_b64,
                    suffix,
                } => {
                    out.extend_from_slice(prefix);
                    let mut buf = itoa_decimal(*counter);
                    out.append(&mut buf);
                    out.push(b'V');
                    out.extend_from_slice(blob_b64.as_bytes());
                    out.extend_from_slice(suffix);
                }
            }
        }
    }

    /// Convenience: serialize into a fresh `Vec<u8>`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1024);
        self.serialize(&mut out);
        out
    }
}

/// Split `bytes` into line slices that include their terminator (LF or CRLF
/// or, at end-of-file, no terminator).
fn split_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    LineIter { bytes, pos: 0 }
}

struct LineIter<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for LineIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
            self.pos += 1;
        }
        if self.pos < self.bytes.len() {
            self.pos += 1; // include the '\n'
        }
        Some(&self.bytes[start..self.pos])
    }
}

/// Parse a single line into a `Line::Identity` if it carries an
/// `identity="…V…"` assignment, else return `None`.
fn try_parse_identity_line(raw: &[u8]) -> Option<Line> {
    // strip leading whitespace, but remember it for prefix
    let mut i = 0;
    while i < raw.len() && matches!(raw[i], b' ' | b'\t') {
        i += 1;
    }
    let key = b"identity";
    if raw.len() < i + key.len() || &raw[i..i + key.len()] != key {
        return None;
    }
    let mut j = i + key.len();
    while j < raw.len() && (raw[j] == b' ' || raw[j] == b'\t') {
        j += 1;
    }
    if j >= raw.len() || raw[j] != b'=' {
        return None;
    }
    j += 1;
    while j < raw.len() && (raw[j] == b' ' || raw[j] == b'\t') {
        j += 1;
    }
    if j >= raw.len() || raw[j] != b'"' {
        return None;
    }
    // prefix = raw[..=j], i.e. up to and including the opening quote
    let prefix_end = j + 1;
    let closing = raw[prefix_end..].iter().position(|b| *b == b'"')?;
    let value = &raw[prefix_end..prefix_end + closing];
    let v_pos = value.iter().position(|b| *b == b'V')?;
    let counter_bytes = &value[..v_pos];
    let blob_bytes = &value[v_pos + 1..];
    if counter_bytes.is_empty() || !counter_bytes.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let counter_str = std::str::from_utf8(counter_bytes).ok()?;
    let counter: u64 = counter_str.parse().ok()?;
    let blob_b64 = std::str::from_utf8(blob_bytes).ok()?.to_owned();
    let prefix = raw[..prefix_end].to_vec();
    let suffix = raw[prefix_end + closing..].to_vec(); // starts with closing `"`
    Some(Line::Identity {
        prefix,
        counter,
        blob_b64,
        suffix,
    })
}

fn itoa_decimal(mut n: u64) -> Vec<u8> {
    if n == 0 {
        return vec![b'0'];
    }
    let mut buf = [0u8; 20];
    let mut i = 20;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    buf[i..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[Identity]
id=Kernel-Error
identity=\"42Vr4FEM/ERFubjCxz6qh/yTZapjpx4UmRSQ34gegxCbGAtXXN1VgICMSxGCzReDAEACFkCfh9hVxcDZH0GcX0BBgAaRghzJEwAVjdeIA44Ki9fYFwePnZpSVopXV5oDEdbFx9kXCtCd0NJUUR5OXFXMDZaV1hIY25tY21FQnhFa3dFRjJ6dDdSUklKY2pSU0Ixa3dSNHhnPT0=\"
nickname=Kernel-Error
phonetic_nickname=
";

    #[test]
    fn parses_identity_line() {
        let f = IdentityFile::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(f.counter(), 42);
        assert!(f.blob_b64().starts_with("r4FEM/ERFubjCxz6qh"));
    }

    #[test]
    fn roundtrips_byte_identical_when_unchanged() {
        let f = IdentityFile::parse(SAMPLE.as_bytes()).unwrap();
        let out = f.to_bytes();
        assert_eq!(out, SAMPLE.as_bytes());
    }

    #[test]
    fn changes_only_counter_digits() {
        let original_blob = IdentityFile::parse(SAMPLE.as_bytes())
            .unwrap()
            .blob_b64()
            .to_owned();
        let mut f = IdentityFile::parse(SAMPLE.as_bytes()).unwrap();
        f.set_counter(99999);
        let out = f.to_bytes();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("identity=\"99999V"));
        assert!(s.contains("nickname=Kernel-Error"));
        // The blob is preserved byte-for-byte.
        let f2 = IdentityFile::parse(&out).unwrap();
        assert_eq!(f2.blob_b64(), original_blob);
        // The rest of the file is unchanged: only the counter digits differ.
        let expected = SAMPLE.replace("\"42V", "\"99999V");
        assert_eq!(s, expected.as_str());
    }

    #[test]
    fn handles_crlf() {
        let crlf = SAMPLE.replace('\n', "\r\n");
        let f = IdentityFile::parse(crlf.as_bytes()).unwrap();
        assert_eq!(f.counter(), 42);
        assert_eq!(f.to_bytes(), crlf.as_bytes());
    }

    #[test]
    fn rejects_file_without_identity() {
        let bad = b"[Identity]\nid=foo\nnickname=bar\n";
        let err = IdentityFile::parse(bad).unwrap_err();
        assert!(matches!(err, Error::NoIdentityKey), "{err:?}");
    }

    #[test]
    fn rejects_non_digit_counter() {
        let bad = "[Identity]\nidentity=\"abcVxyz\"\n";
        let err = IdentityFile::parse(bad.as_bytes()).unwrap_err();
        // It is rejected at the line-classification stage (no Identity match),
        // hence shows up as NoIdentityKey.
        assert!(matches!(err, Error::NoIdentityKey), "{err:?}");
    }

    #[test]
    fn preserves_leading_whitespace_and_no_trailing_newline() {
        let s = "  identity=\"7Vblob\"";
        let f = IdentityFile::parse(s.as_bytes()).unwrap();
        assert_eq!(f.counter(), 7);
        assert_eq!(f.to_bytes(), s.as_bytes());
    }

    #[test]
    fn extracts_nickname_id_and_phonetic_nickname() {
        let f = IdentityFile::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(f.nickname(), Some("Kernel-Error"));
        assert_eq!(f.local_id(), Some("Kernel-Error"));
        assert_eq!(f.phonetic_nickname(), Some(""));
    }

    #[test]
    fn missing_optional_fields_return_none() {
        // .ini with only the identity= line — no nickname / id / phonetic.
        let minimal = "[Identity]\nidentity=\"5Vxyz\"\n";
        let f = IdentityFile::parse(minimal.as_bytes()).unwrap();
        assert_eq!(f.nickname(), None);
        assert_eq!(f.local_id(), None);
        assert_eq!(f.phonetic_nickname(), None);
    }

    #[test]
    fn itoa_corner_cases() {
        assert_eq!(itoa_decimal(0), b"0");
        assert_eq!(itoa_decimal(1), b"1");
        assert_eq!(itoa_decimal(10), b"10");
        assert_eq!(itoa_decimal(u64::MAX), b"18446744073709551615");
    }
}
