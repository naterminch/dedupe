use std::collections::HashMap;
use std::path::Path;

/// Minimal glob matcher: `**` crosses separators, `*` and `?` do not.
#[must_use]
pub fn glob_match(pattern: &str, path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = if pattern.contains('/') || pattern.contains('\\') {
        text.as_str()
    } else {
        name.as_str()
    };
    let pat = pattern.replace('\\', "/").to_ascii_lowercase();
    let target = target.to_ascii_lowercase();
    if glob_bytes(pat.as_bytes(), target.as_bytes()) {
        return true;
    }
    if pat.contains('/') {
        for (i, b) in target.bytes().enumerate() {
            if b == b'/' && glob_bytes(pat.as_bytes(), &target.as_bytes()[i + 1..]) {
                return true;
            }
        }
    }
    false
}

fn glob_bytes(pattern: &[u8], text: &[u8]) -> bool {
    fn go(
        p: &[u8],
        t: &[u8],
        pi: usize,
        ti: usize,
        memo: &mut HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(v) = memo.get(&(pi, ti)) {
            return *v;
        }
        let result = if pi == p.len() {
            ti == t.len()
        } else if p[pi] == b'*' {
            if pi + 1 < p.len() && p[pi + 1] == b'*' {
                let next = pi + 2;
                go(p, t, next, ti, memo) || (ti < t.len() && go(p, t, pi, ti + 1, memo))
            } else {
                go(p, t, pi + 1, ti, memo)
                    || (ti < t.len() && t[ti] != b'/' && go(p, t, pi, ti + 1, memo))
            }
        } else if pi < p.len() && p[pi] == b'?' {
            ti < t.len() && t[ti] != b'/' && go(p, t, pi + 1, ti + 1, memo)
        } else {
            ti < t.len() && p[pi] == t[ti] && go(p, t, pi + 1, ti + 1, memo)
        };
        memo.insert((pi, ti), result);
        result
    }
    let mut memo: HashMap<(usize, usize), bool> = HashMap::new();
    go(pattern, text, 0, 0, &mut memo)
}
