//! A small JSON reader (RFC 8259), enough to read the answer of the release service
//! (`update`). No dependency, no writing. Objects keep their members in order; numbers are
//! kept as `f64` (the release answer only needs sizes, well below 2^53). Nesting is limited
//! so that a hostile answer cannot exhaust the stack.

/// Deepest nesting of arrays and objects accepted.
const MAX_DEPTH: usize = 32;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string.
    Text(String),
    /// An array.
    Array(Vec<Value>),
    /// An object, its members in the order of the text.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The member `key` of an object (the first one, if repeated).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The text of a string value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The value of a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// A number that is a whole, non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 && *n < 9.0e15 => Some(*n as u64),
            _ => None,
        }
    }

    /// The elements of an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }
}

/// Reads a JSON text. `None` when it is not valid JSON (or nests too deeply).
pub fn parse(text: &str) -> Option<Value> {
    let mut reader = Reader { bytes: text.as_bytes(), at: 0 };
    let value = reader.value(0)?;
    reader.space();
    (reader.at == reader.bytes.len()).then_some(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn literal(&mut self, word: &[u8], value: Value) -> Option<Value> {
        let end = self.at.checked_add(word.len())?;
        (self.bytes.get(self.at..end)? == word).then(|| {
            self.at = end;
            value
        })
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        self.space();
        match self.peek()? {
            b'n' => self.literal(b"null", Value::Null),
            b't' => self.literal(b"true", Value::Bool(true)),
            b'f' => self.literal(b"false", Value::Bool(false)),
            b'"' => self.string().map(Value::Text),
            b'[' if depth < MAX_DEPTH => self.array(depth + 1),
            b'{' if depth < MAX_DEPTH => self.object(depth + 1),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn array(&mut self, depth: usize) -> Option<Value> {
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.peek()? == b']' {
            self.at += 1;
            return Some(Value::Array(items));
        }
        loop {
            items.push(self.value(depth)?);
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(Value::Array(items));
                }
                _ => return None,
            }
        }
    }

    fn object(&mut self, depth: usize) -> Option<Value> {
        self.at += 1;
        let mut members = Vec::new();
        self.space();
        if self.peek()? == b'}' {
            self.at += 1;
            return Some(Value::Object(members));
        }
        loop {
            self.space();
            if self.peek()? != b'"' {
                return None;
            }
            let key = self.string()?;
            self.space();
            if self.peek()? != b':' {
                return None;
            }
            self.at += 1;
            members.push((key, self.value(depth)?));
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return Some(Value::Object(members));
                }
                _ => return None,
            }
        }
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        let digits = |r: &mut Reader<'_>| {
            let from = r.at;
            while r.peek().is_some_and(|b| b.is_ascii_digit()) {
                r.at += 1;
            }
            r.at - from
        };
        match self.peek()? {
            b'0' => self.at += 1,
            b'1'..=b'9' => {
                digits(self);
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if digits(self) == 0 {
                return None;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                return None;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).ok()?;
        text.parse().ok().map(Value::Number)
    }

    fn hex4(&mut self) -> Option<u32> {
        let end = self.at.checked_add(4)?;
        let text = std::str::from_utf8(self.bytes.get(self.at..end)?).ok()?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        self.at = end;
        u32::from_str_radix(text, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while self.peek().is_some_and(|b| b != b'"' && b != b'\\' && b >= 0x20) {
                self.at += 1;
            }
            out.push_str(std::str::from_utf8(&self.bytes[start..self.at]).ok()?);
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.at += 1;
                    let escape = self.peek()?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let first = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&first) {
                                // A surrogate pair: the low half must follow.
                                if self.bytes.get(self.at..self.at + 2)? != b"\\u" {
                                    return None;
                                }
                                self.at += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return None;
                                }
                                0x10000 + ((first - 0xD800) << 10) + (low - 0xDC00)
                            } else {
                                first
                            };
                            out.push(char::from_u32(code)?);
                        }
                        _ => return None,
                    }
                }
                // A control character inside a string.
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_value() {
        let value =
            parse(r#" {"a": [1, -2.5, 3e2, true, false, null], "b": {"c": "d"}} "#).expect("valid");
        let a = value.get("a").and_then(Value::as_array).expect("array");
        assert_eq!(a.len(), 6);
        assert_eq!(a[0].as_u64(), Some(1));
        assert_eq!(a[1], Value::Number(-2.5));
        assert_eq!(a[2].as_u64(), Some(300));
        assert_eq!(a[3].as_bool(), Some(true));
        assert_eq!(a[5], Value::Null);
        assert_eq!(value.get("b").and_then(|b| b.get("c")).and_then(Value::as_str), Some("d"));
        assert_eq!(parse("[]"), Some(Value::Array(Vec::new())));
        assert_eq!(parse("{}"), Some(Value::Object(Vec::new())));
    }

    #[test]
    fn reads_escapes_and_unicode() {
        let value = parse(r#""a\"b\\c\/d\n\u00e9\ud83d\ude00é""#).expect("valid");
        assert_eq!(value.as_str(), Some("a\"b\\c/d\né😀é"));
    }

    #[test]
    fn refuses_what_is_not_json() {
        for text in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "{\"a\":1,}",
            "01",
            "1.",
            "-",
            "\"a",
            "\"\u{1}\"",
            "\"\\x\"",
            "\"\\ud800\"",
            "\"\\ud800\\u0041\"",
            "tru",
            "[1] 2",
            "nul",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn deep_nesting_is_refused_not_overflowed() {
        let deep = "[".repeat(100_000);
        assert_eq!(parse(&deep), None);
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(&ok).is_some());
        let too_deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert_eq!(parse(&too_deep), None);
    }

    #[test]
    fn as_u64_only_takes_whole_non_negative_numbers() {
        assert_eq!(Value::Number(2.5).as_u64(), None);
        assert_eq!(Value::Number(-1.0).as_u64(), None);
        assert_eq!(Value::Text("1".into()).as_u64(), None);
        assert_eq!(Value::Number(1_234_567.0).as_u64(), Some(1_234_567));
    }
}
