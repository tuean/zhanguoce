//! HTML escaping — every piece of data and every piece of user input goes
//! through here before it is concatenated into a page.

/// Escape ampersand, angle brackets and both quote characters so the result is
/// safe in element text and in double- or single-quoted attributes.
pub fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Escape the text and wrap each case-insensitive literal occurrence of the
/// needle in a mark element — used for search snippets, which come straight
/// from the corpus and from the user's query.
pub fn escape_highlight(text: &str, needle: &str) -> String {
    let needle: Vec<char> = needle.trim().chars().map(fold).collect();
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 16);
    let mut index = 0;
    while index < chars.len() {
        if !needle.is_empty() && starts_with_folded(&chars[index..], &needle) {
            out.push_str("<mark>");
            let end = index + needle.len();
            out.push_str(&escape(&chars[index..end].iter().collect::<String>()));
            out.push_str("</mark>");
            index = end;
        } else {
            out.push_str(&escape(&chars[index].to_string()));
            index += 1;
        }
    }
    out
}

fn starts_with_folded(haystack: &[char], needle: &[char]) -> bool {
    haystack.len() >= needle.len()
        && needle
            .iter()
            .zip(haystack)
            .all(|(expected, actual)| fold(*actual) == *expected)
}

fn fold(character: char) -> char {
    character.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_every_dangerous_character() {
        assert_eq!(escape("<b>&\"'"), "&lt;b&gt;&amp;&quot;&#39;");
        assert_eq!(escape("衞鞅亡魏入秦"), "衞鞅亡魏入秦");
        assert_eq!(escape(""), "");
    }

    #[test]
    fn highlight_wraps_the_literal_match_only() {
        assert_eq!(
            escape_highlight("…衞鞅亡魏…", "衞鞅"),
            "…<mark>衞鞅</mark>亡魏…"
        );
        // The corpus index folds ASCII case, so the highlighter does too.
        assert_eq!(
            escape_highlight("abcABC", "ABC"),
            "<mark>abc</mark><mark>ABC</mark>"
        );
        assert_eq!(escape_highlight("no match", "zzz"), "no match");
        assert_eq!(escape_highlight("x", ""), "x");
    }

    #[test]
    fn highlight_escapes_before_marking() {
        assert_eq!(
            escape_highlight("<script>alert(1)</script>", "alert"),
            "&lt;script&gt;<mark>alert</mark>(1)&lt;/script&gt;"
        );
    }

    #[test]
    fn highlight_never_splits_a_multibyte_character() {
        assert_eq!(escape_highlight("戰國策", "國策"), "戰<mark>國策</mark>");
    }
}
