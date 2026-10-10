use std::collections::BTreeMap;

/// Minimal `key=value` line store used for state files. Values may not
/// contain newlines.
#[derive(Default, Clone)]
pub struct Kv(BTreeMap<String, String>);

impl Kv {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn parse(text: &str) -> Self {
        let mut m = BTreeMap::new();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        Kv(m)
    }
    pub fn render(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.0 {
            s.push_str(k);
            s.push('=');
            s.push_str(v);
            s.push('\n');
        }
        s
    }
    pub fn set(&mut self, k: &str, v: impl ToString) {
        self.0.insert(k.to_string(), v.to_string().replace('\n', " "));
    }
    pub fn get(&self, k: &str) -> Option<&str> {
        self.0.get(k).map(|s| s.as_str())
    }
    pub fn f32(&self, k: &str) -> Option<f32> {
        self.get(k)?.parse().ok()
    }
    pub fn usize(&self, k: &str) -> Option<usize> {
        self.get(k)?.parse().ok()
    }
    pub fn bool(&self, k: &str) -> Option<bool> {
        match self.get(k)? {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }
    pub fn set_bool(&mut self, k: &str, v: bool) {
        self.set(k, if v { "1" } else { "0" });
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

pub fn from_hex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut out = Vec::with_capacity(b.len() / 2);
    for pair in b.chunks_exact(2) {
        match (nib(pair[0]), nib(pair[1])) {
            (Some(h), Some(l)) => out.push(h << 4 | l),
            _ => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut k = Kv::new();
        k.set("bpm", 123.5);
        k.set_bool("on", true);
        k.set("name", "a=b");
        let k2 = Kv::parse(&k.render());
        assert_eq!(k2.f32("bpm"), Some(123.5));
        assert_eq!(k2.bool("on"), Some(true));
        assert_eq!(k2.get("name"), Some("a=b"));
    }

    #[test]
    fn hex() {
        let v = vec![0u8, 1, 127, 255];
        assert_eq!(from_hex(&to_hex(&v)), v);
    }
}
