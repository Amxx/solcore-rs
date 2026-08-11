/// Decodes the source spelling of a Solcore string literal.
///
/// Unknown escapes deliberately discard the backslash and retain the escaped
/// character. This is the evaluator's established semantics and backend
/// consumers must use the same decoding before materializing literal bytes.
pub fn decode_string_literal(text: &str) -> Option<String> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            other => out.push(other),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode_string_literal;

    #[test]
    fn decodes_known_escapes() {
        assert_eq!(
            decode_string_literal(r#""a\n\r\t\"\\b""#),
            Some("a\n\r\t\"\\b".to_owned())
        );
    }

    #[test]
    fn preserves_evaluator_semantics_for_unknown_escapes() {
        assert_eq!(
            decode_string_literal(r#""before\qafter""#),
            Some("beforeqafter".to_owned())
        );
    }

    #[test]
    fn rejects_malformed_literal_spellings() {
        assert_eq!(decode_string_literal("not quoted"), None);
        assert_eq!(decode_string_literal("\"trailing\\\""), None);
    }
}
