use regex::Regex;

pub fn markdown_to_plain_text(text: &str) -> String {
    let mut s = text.to_string();
    let replacements = [
        (r"(?s)```[^\n]*\n?(.*?)```", "$1"),
        (r"!\[[^\]]*\]\([^)]*\)", ""),
        (r"\[([^\]]+)\]\([^)]*\)", "$1"),
        (r"(?m)^\|[\s:|\-]+\|$", ""),
        (r"(?m)^#{1,6}\s+", ""),
        (r"\*\*(.+?)\*\*|__(.+?)__", "$1$2"),
        (r"~~(.+?)~~", "$1"),
        (r"(?m)^>\s?", ""),
        (r"(?m)^[-*_]{3,}\s*$", ""),
        (r"`([^`]+)`", "$1"),
    ];
    for (pattern, replacement) in replacements {
        s = Regex::new(pattern)
            .unwrap()
            .replace_all(&s, replacement)
            .into_owned();
    }
    s = Regex::new(r"(?m)^(\s*)[-*+]\s+")
        .unwrap()
        .replace_all(&s, "${1}• ")
        .into_owned();
    s = Regex::new(r"(?m)^\|(.+)\|$")
        .unwrap()
        .replace_all(&s, |caps: &regex::Captures<'_>| {
            caps[1]
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("  ")
        })
        .into_owned();
    Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(&s, "\n\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_markdown() {
        assert_eq!(
            markdown_to_plain_text("## Hi\n[OpenAI](https://openai.com)"),
            "Hi\nOpenAI"
        );
        assert_eq!(
            markdown_to_plain_text("```rust\nfn main() {}\n```"),
            "fn main() {}"
        );
    }
}
