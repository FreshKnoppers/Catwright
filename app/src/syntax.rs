//! A tiny single-pass tokenizer for code blocks: keywords, strings,
//! comments and numbers — one color per token type. No grammar, no tree:
//! good enough for reading, honest about what it is.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    Str,
    Comment,
    Num,
    Plain,
}

fn keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "rust" => &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while",
        ],
        "python" | "py" => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
            "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
            "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return",
            "True", "try", "while", "with", "yield",
        ],
        "javascript" | "js" | "typescript" | "ts" => &[
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "default",
            "delete",
            "do",
            "else",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "let",
            "new",
            "null",
            "of",
            "return",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "undefined",
            "var",
            "void",
            "while",
            "yield",
        ],
        "bash" | "sh" | "shell" | "zsh" => &[
            "case", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if", "in",
            "then", "until", "while",
        ],
        "json" => &["false", "null", "true"],
        "go" => &[
            "break",
            "case",
            "chan",
            "const",
            "continue",
            "default",
            "defer",
            "else",
            "fallthrough",
            "for",
            "func",
            "go",
            "goto",
            "if",
            "import",
            "interface",
            "map",
            "package",
            "range",
            "return",
            "select",
            "struct",
            "switch",
            "type",
            "var",
        ],
        _ => &[],
    }
}

/// Line-comment starters by language (no block comments — kept tiny).
fn line_comment(lang: &str) -> Option<&'static str> {
    match lang {
        "python" | "py" | "bash" | "sh" | "shell" | "zsh" | "toml" | "yaml" | "yml" => Some("#"),
        "lua" | "sql" => Some("--"),
        "" => None,
        _ => Some("//"),
    }
}

/// Tokenize into runs: neighbouring same-type tokens are merged, and the
/// concatenation of all run texts is exactly `code` (newlines included).
/// The scan walks whole `char`s, so multi-byte text outside strings and
/// comments (umlauts, emojis) keeps its exact bytes instead of being
/// rebuilt from single bytes as Latin-1.
pub fn tokenize(lang: &str, code: &str) -> Vec<(Tok, String)> {
    let keywords = keywords(lang);
    let comment = line_comment(lang);
    let bytes = code.as_bytes();
    let mut out: Vec<(Tok, String)> = Vec::new();
    let mut word = String::new();
    let mut i = 0;
    macro_rules! flush_word {
        () => {
            if !word.is_empty() {
                let tok = if keywords.contains(&word.as_str()) {
                    Tok::Keyword
                } else if word.bytes().next().is_some_and(|b| b.is_ascii_digit())
                    && word.bytes().all(|b| b.is_ascii_digit())
                {
                    Tok::Num
                } else {
                    Tok::Plain
                };
                push_run(&mut out, tok, &std::mem::take(&mut word));
            }
        };
    }
    while let Some(ch) = code[i..].chars().next() {
        // comments run to the end of the line: the starter is ASCII, and
        // the scan stops at the single-byte '\n', a char boundary
        if let Some(starter) = comment {
            let sc = starter.as_bytes();
            if bytes[i..].starts_with(sc) {
                flush_word!();
                let start = i;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                push_run(&mut out, Tok::Comment, &code[start..i]);
                continue;
            }
        }
        match ch {
            '"' | '\'' | '`' => {
                flush_word!();
                let start = i;
                i += ch.len_utf8();
                while i < code.len() {
                    let c = code[i..].chars().next().unwrap();
                    i += c.len_utf8();
                    if c == '\\' {
                        // the escaped CHARACTER is skipped whole: a byte
                        // skip would land inside a multi-byte one
                        if let Some(escaped) = code[i..].chars().next() {
                            i += escaped.len_utf8();
                        }
                        continue;
                    }
                    if c == ch {
                        break;
                    }
                }
                push_run(&mut out, Tok::Str, &code[start..i]);
            }
            c if c.is_ascii_alphanumeric() || c == '_' => {
                word.push(c);
                i += c.len_utf8();
            }
            c => {
                flush_word!();
                let mut buf = [0u8; 4];
                push_run(&mut out, Tok::Plain, c.encode_utf8(&mut buf));
                i += c.len_utf8();
            }
        }
    }
    flush_word!();
    out
}

fn push_run(out: &mut Vec<(Tok, String)>, tok: Tok, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some((last_tok, last)) = out.last_mut() {
        if *last_tok == tok {
            last.push_str(text);
            return;
        }
    }
    out.push((tok, text.to_string()));
}

type LineTokens = Vec<Vec<(Tok, String)>>;

pub fn tokenize_lines_cached(lang: &str, code: &str) -> std::sync::Arc<LineTokens> {
    static CACHE: std::sync::OnceLock<crate::memo::Memo<LineTokens>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| crate::memo::Memo::new(4))
        .get_or_insert(crate::memo::hash_pair(Some(lang), code), || {
            code.lines().map(|line| tokenize(lang, line)).collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(code: &str, lang: &str) -> Vec<Tok> {
        tokenize(lang, code).iter().map(|(t, _)| *t).collect()
    }

    #[test]
    fn runs_join_back_to_the_source() {
        let code = "let x = \"hi\\n\"; // comment\nfn main() {}";
        let joined: String = tokenize("rust", code)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(joined, code);
    }

    #[test]
    fn rust_tokens_split_into_kinds() {
        let code = "let s = \"a\"; // c\nreturn 42;";
        assert_eq!(
            kinds(code, "rust"),
            vec![
                Tok::Keyword, // let
                Tok::Plain,   // "s = "
                Tok::Str,
                Tok::Plain, // "; "
                Tok::Comment,
                Tok::Plain,   // "\n"
                Tok::Keyword, // return
                Tok::Plain,
                Tok::Num,
                Tok::Plain,
            ]
        );
    }

    #[test]
    fn python_comments_use_hash_and_strings_keep_escapes() {
        let code = "x = \"a\\\"b\" # nope\nif True:";
        let joined: String = tokenize("python", code)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(joined, code);
        assert_eq!(kinds("# c\n", "python"), vec![Tok::Comment, Tok::Plain]);
        assert_eq!(
            kinds("if True:", "python"),
            vec![Tok::Keyword, Tok::Plain, Tok::Keyword, Tok::Plain]
        );
    }

    #[test]
    fn unknown_language_has_no_keywords_or_comments() {
        assert_eq!(kinds("foo bar # x", "worf"), vec![Tok::Plain]);
        assert_eq!(
            kinds("true null", "json"),
            vec![Tok::Keyword, Tok::Plain, Tok::Keyword]
        );
    }

    #[test]
    fn tokenize_lines_cached_reuses_one_entry_per_content() {
        let a = tokenize_lines_cached("rust", "let x = 1;\n\nlet y = 2;");
        let b = tokenize_lines_cached("rust", "let x = 1;\n\nlet y = 2;");
        assert!(std::sync::Arc::ptr_eq(&a, &b), "same content, same entry");
        assert_eq!(a.len(), 3);
        let per_line: Vec<Vec<Tok>> = a
            .iter()
            .map(|l| l.iter().map(|(t, _)| *t).collect())
            .collect();
        assert_eq!(
            per_line,
            vec![
                vec![Tok::Keyword, Tok::Plain, Tok::Num, Tok::Plain],
                Vec::<Tok>::new(),
                vec![Tok::Keyword, Tok::Plain, Tok::Num, Tok::Plain],
            ]
        );
        let other = tokenize_lines_cached("rust", "let x = 1;");
        assert!(!std::sync::Arc::ptr_eq(&a, &other));
        let other_lang = tokenize_lines_cached("python", "let x = 1;\n\nlet y = 2;");
        assert!(
            !std::sync::Arc::ptr_eq(&a, &other_lang),
            "lang is part of the key"
        );
    }

    #[test]
    fn multi_byte_text_stays_intact_outside_strings_and_comments() {
        // umlauts and an emoji in plain code, a string and a comment: the
        // runs must join back to the exact bytes (rebuilt-from-bytes runs
        // used to turn "ö" into "Ã¶")
        let code = "let größe = \"Grüße ✅\"; // größer\nlet ünion_ = 'ü';";
        let joined: String = tokenize("rust", code)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(joined, code);
        // a backslash skips the escaped CHARACTER whole: `\ü` may not
        // split the two-byte ü
        let escaped = "s = \"a\\ü\" # √";
        let joined: String = tokenize("python", escaped)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect();
        assert_eq!(joined, escaped);
    }
}
