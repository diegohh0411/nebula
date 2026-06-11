pub fn normalize(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

pub fn like_pattern(query: &str) -> Option<String> {
    let norm = normalize(query);
    if norm.is_empty() {
        return None;
    }
    let mut pat = String::from("%");
    for c in norm.chars() {
        match c {
            '\\' | '%' | '_' => {
                pat.push('\\');
                pat.push(c);
            }
            ' ' => pat.push('%'),
            other => pat.push(other),
        }
    }
    pat.push('%');
    Some(pat)
}

pub(crate) fn matches_tokens(haystack: &str, query_norm: &str) -> bool {
    let mut pos = 0;
    for tok in query_norm.split_whitespace() {
        match haystack[pos..].find(tok) {
            Some(i) => pos += i + tok.len(),
            None => return false,
        }
    }
    true
}
