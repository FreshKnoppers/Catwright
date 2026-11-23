//! Minimal hand-rolled Markdown parser (no extra crates): headings, paragraphs,
//! fenced code blocks, lists, quotes, hr, pipe tables, and inline
//! bold/italic/code/links. Output is a block list that render.rs paints with
//! egui.

#[derive(Clone, Debug, PartialEq)]
pub enum MdBlock {
    Heading(u8, Vec<MdInline>),
    Para(Vec<MdInline>),
    /// `closed`: the closing fence was seen (false while an answer still streams).
    Code {
        lang: String,
        code: String,
        closed: bool,
    },
    List(Vec<Vec<MdInline>>),
    Quote(Vec<MdInline>),
    /// Pipe table: header cells and body cells, each parsed as inline
    /// markdown. The `|---|`-style separator row is consumed; its optional
    /// alignment colons are accepted but not honoured (columns of numbers
    /// render right-aligned, all others left-aligned, like the
    /// generative-ui `table` node). Body rows are
    /// padded or truncated to the header's cell count.
    Table {
        columns: Vec<Vec<MdInline>>,
        rows: Vec<Vec<Vec<MdInline>>>,
    },
    Hr,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MdInline {
    Text(String),
    Bold(Vec<MdInline>),
    Italic(Vec<MdInline>),
    Code(String),
    Link { text: String, url: String },
}

/// The transcript redraws every frame; the blocks of an agent message do
/// not change with it. This cache hands the parsed blocks back keyed by
/// the text's hash (cleared wholesale once it is full).
pub fn parse_cached(input: &str) -> std::sync::Arc<Vec<MdBlock>> {
    static CACHE: std::sync::OnceLock<crate::memo::Memo<Vec<MdBlock>>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| crate::memo::Memo::new(1024))
        .get_or_insert(crate::memo::hash_str(input), || parse(input))
}

pub fn parse(input: &str) -> Vec<MdBlock> {
    // First split off fenced code blocks so their content is never parsed.
    // A fence is a line of its own, as in CommonMark: backticks inside a
    // line (an inline "` ```ui-js `" in prose or a table cell) are text and
    // must not swallow the rest of the message into a code block.
    let mut blocks = Vec::new();
    // start of the text run since the last fence
    let mut text_start = 0;
    let mut pos = 0;
    while pos < input.len() {
        let line_end = input[pos..].find('\n').map_or(input.len(), |n| pos + n + 1);
        let line = &input[pos..line_end];
        let Some((ticks, info)) = opening_fence(line) else {
            pos = line_end;
            continue;
        };
        blocks.extend(parse_text(&input[text_start..pos]));
        // the info string names the language once its line is complete;
        // while an answer streams the tag may still be cut off
        let lang = if line.ends_with('\n') {
            info.to_string()
        } else {
            String::new()
        };
        let body_start = line_end;
        let mut close = None;
        let mut p = body_start;
        while p < input.len() {
            let end = input[p..].find('\n').map_or(input.len(), |n| p + n + 1);
            if closing_fence(&input[p..end], ticks) {
                close = Some((p, end));
                break;
            }
            p = end;
        }
        match close {
            Some((code_end, after)) => {
                blocks.push(MdBlock::Code {
                    lang,
                    code: input[body_start..code_end]
                        .trim_end_matches('\n')
                        .to_string(),
                    closed: true,
                });
                pos = after;
                text_start = after;
            }
            None => {
                blocks.push(MdBlock::Code {
                    lang,
                    code: input[body_start..].trim_end_matches('\n').to_string(),
                    closed: false,
                });
                return blocks;
            }
        }
    }
    blocks.extend(parse_text(&input[text_start..]));
    blocks
}

/// An opening code fence: at least three backticks at the start of the
/// line (after indentation, so fences inside list items count) and an info
/// string without backticks. Returns the fence length and the info string.
fn opening_fence(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let ticks = t.chars().take_while(|c| *c == '`').count();
    if ticks < 3 {
        return None;
    }
    let info = t[ticks..].trim();
    (!info.contains('`')).then_some((ticks, info))
}

/// A closing fence for an opening fence of `ticks` backticks: a line of
/// nothing but at least as many backticks.
fn closing_fence(line: &str, ticks: usize) -> bool {
    let t = line.trim();
    t.len() >= ticks && t.chars().all(|c| c == '`')
}

fn parse_text(text: &str) -> Vec<MdBlock> {
    let mut blocks = Vec::new();
    let mut para: Vec<MdInline> = Vec::new();
    let mut list: Option<Vec<Vec<MdInline>>> = None;
    let mut quote: Vec<MdInline> = Vec::new();

    fn flush_para(para: &mut Vec<MdInline>, blocks: &mut Vec<MdBlock>) {
        if !para.is_empty() {
            blocks.push(MdBlock::Para(std::mem::take(para)));
        }
    }
    fn flush_list(list: &mut Option<Vec<Vec<MdInline>>>, blocks: &mut Vec<MdBlock>) {
        if let Some(items) = list.take() {
            if !items.is_empty() {
                blocks.push(MdBlock::List(items));
            }
        }
    }
    fn flush_quote(quote: &mut Vec<MdInline>, blocks: &mut Vec<MdBlock>) {
        if !quote.is_empty() {
            blocks.push(MdBlock::Quote(std::mem::take(quote)));
        }
    }

    for line in split_tables(text) {
        match line {
            Line::Table(table) => {
                // a table always splits the surrounding blocks
                flush_para(&mut para, &mut blocks);
                flush_list(&mut list, &mut blocks);
                flush_quote(&mut quote, &mut blocks);
                blocks.push(MdBlock::Table {
                    columns: table.columns,
                    rows: table.rows,
                });
            }
            Line::Text(line) => {
                let t = line.trim_end();
                if t.starts_with("```") {
                    // handled above; a stray fence line is just text
                    para.push(MdInline::Text(t.to_string()));
                    continue;
                }
                if let Some(h) = t.strip_prefix("#") {
                    let level = h.chars().take_while(|c| *c == '#').count().min(6) as u8;
                    let rest = h.trim_start_matches('#').trim();
                    if !rest.is_empty() || level > 0 {
                        flush_para(&mut para, &mut blocks);
                        flush_list(&mut list, &mut blocks);
                        flush_quote(&mut quote, &mut blocks);
                        blocks.push(MdBlock::Heading(level.max(1), parse_inline(rest)));
                        continue;
                    }
                }
                if t == "---" || t == "***" || t == "___" {
                    flush_para(&mut para, &mut blocks);
                    flush_list(&mut list, &mut blocks);
                    flush_quote(&mut quote, &mut blocks);
                    blocks.push(MdBlock::Hr);
                    continue;
                }
                if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                    flush_para(&mut para, &mut blocks);
                    flush_quote(&mut quote, &mut blocks);
                    list.get_or_insert_with(Vec::new).push(parse_inline(item));
                    continue;
                }
                if let Some(q) = t.strip_prefix("> ") {
                    flush_para(&mut para, &mut blocks);
                    flush_list(&mut list, &mut blocks);
                    quote.extend(parse_inline(q));
                    quote.push(MdInline::Text("\n".into()));
                    continue;
                }
                if t.is_empty() {
                    flush_para(&mut para, &mut blocks);
                    flush_list(&mut list, &mut blocks);
                    flush_quote(&mut quote, &mut blocks);
                    continue;
                }
                flush_list(&mut list, &mut blocks);
                flush_quote(&mut quote, &mut blocks);
                if !para.is_empty() {
                    para.push(MdInline::Text("\n".into()));
                }
                para.extend(parse_inline(t));
            }
        }
    }
    flush_para(&mut para, &mut blocks);
    flush_list(&mut list, &mut blocks);
    flush_quote(&mut quote, &mut blocks);
    blocks
}

// ---------------------------------------------------------------------------
// Pipe tables (GFM style): a header row, a |---| separator and body rows,
// lifted out of the line stream before the block rules run.
// ---------------------------------------------------------------------------

/// One input line, or a complete table found by `split_tables`.
enum Line<'a> {
    Text(&'a str),
    Table(Table),
}

/// A parsed pipe table: header cells and body rows, one inline run per cell.
struct Table {
    columns: Vec<Vec<MdInline>>,
    rows: Vec<Vec<Vec<MdInline>>>,
}

/// Pre-pass of `parse_text`: walk the input lines and lift out complete pipe
/// tables (a table needs a header row and a fitting `|---|`-style separator
/// row right after it; a lone `| a | b |` stays a text line). All other lines
/// pass through unchanged, so the block rules in `parse_text` run as before.
fn split_tables(text: &str) -> Vec<Line<'_>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((table, next)) = table_at(&lines, i) {
            out.push(Line::Table(table));
            i = next;
        } else {
            out.push(Line::Text(lines[i]));
            i += 1;
        }
    }
    out
}

/// The table starting at `lines[i]`, if there is one: its parsed cells and
/// the index of the first line after it. Body rows continue while lines are
/// non-empty, contain a pipe and do not start another block.
fn table_at(lines: &[&str], i: usize) -> Option<(Table, usize)> {
    let header = lines[i].trim_end();
    if !header.contains('|') || starts_other_block(header) {
        return None;
    }
    let columns = split_row(header);
    if columns.is_empty() || i + 1 >= lines.len() {
        return None;
    }
    if separator_cell_count(lines[i + 1]) != Some(columns.len()) {
        return None;
    }
    let mut j = i + 2;
    let mut raw_rows = Vec::new();
    while j < lines.len() {
        let row = lines[j].trim_end();
        if row.is_empty() || !row.contains('|') || starts_other_block(row) {
            break;
        }
        raw_rows.push(split_row(row));
        j += 1;
    }
    let columns: Vec<Vec<MdInline>> = columns.iter().map(|c| parse_inline(c)).collect();
    let rows = raw_rows
        .into_iter()
        .map(|mut row| {
            row.resize(columns.len(), String::new()); // pad or truncate
            row.iter().map(|cell| parse_inline(cell)).collect()
        })
        .collect();
    Some((Table { columns, rows }, j))
}

/// Split one table row into cells at unescaped `|`. A leading and a trailing
/// pipe (not escaped) add no cell; `\|` inside a cell becomes a literal `|`.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for ch in t.chars() {
        if escaped {
            if ch == '|' {
                cell.pop(); // keep only the pipe, not the backslash
            }
            cell.push(ch);
            escaped = false;
        } else if ch == '\\' {
            cell.push(ch);
            escaped = true;
        } else if ch == '|' {
            cells.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(ch);
        }
    }
    cells.push(cell.trim().to_string());
    if t.starts_with('|') {
        cells.remove(0); // the empty cell left of the leading pipe
    }
    if t.ends_with('|') && !t.ends_with("\\|") {
        cells.pop(); // the empty cell right of the trailing pipe
    }
    cells
}

/// Cell count if `line` is a `|---|`-style separator row (each cell may carry
/// alignment colons, which are accepted but not honoured), otherwise `None`.
fn separator_cell_count(line: &str) -> Option<usize> {
    let cells = split_row(line);
    if cells.is_empty() || !cells.iter().all(|c| is_separator_cell(c)) {
        return None;
    }
    Some(cells.len())
}

/// One separator cell: `---` with optional colons (`:---`, `---:`, `:---:`).
fn is_separator_cell(cell: &str) -> bool {
    let c = cell.trim();
    let body = c.strip_prefix(':').unwrap_or(c);
    let body = body.strip_suffix(':').unwrap_or(body);
    !body.is_empty() && body.bytes().all(|b| b == b'-')
}

/// Lines that begin another block kind in `parse_text` (fence, heading, hr,
/// list item, quote). They can neither start a table nor continue one.
fn starts_other_block(t: &str) -> bool {
    t.starts_with("```")
        || (t.starts_with('#') && t != "#")
        || t == "---"
        || t == "***"
        || t == "___"
        || t.strip_prefix("- ").is_some()
        || t.strip_prefix("* ").is_some()
        || t.strip_prefix("> ").is_some()
}

/// The next occurrence of `needle` at or after a position of `hay`,
/// remembering the last answer: a search further on reuses it while it is
/// still ahead, and a miss stays a miss. Positions only grow while a line
/// is parsed, so each needle scans the line about once, and the parser is
/// linear instead of quadratic (audit M5: a 64 KiB line of `[` took
/// 0.6 s per parse, every frame while it streamed).
struct Next {
    needle: &'static str,
    /// Where the remembered search started, and what it found.
    from: usize,
    found: Option<usize>,
}

impl Next {
    fn new(needle: &'static str) -> Self {
        Self {
            needle,
            from: usize::MAX,
            found: None,
        }
    }

    fn at(&mut self, hay: &str, p: usize) -> Option<usize> {
        if p >= self.from {
            match self.found {
                None => return None,
                Some(f) if f >= p => return Some(f),
                _ => {}
            }
        }
        self.from = p;
        self.found = hay[p..].find(self.needle).map(|o| p + o);
        self.found
    }
}

/// The next run of exactly n backticks per n, remembered like [`Next`].
#[derive(Default)]
struct Runs(std::collections::HashMap<usize, (usize, Option<usize>)>);

impl Runs {
    /// Start of the first run of exactly `n` backticks at or after `p`
    /// (`p` is never inside a run).
    fn at(&mut self, hay: &str, n: usize, p: usize) -> Option<usize> {
        if let Some(&(from, found)) = self.0.get(&n) {
            if p >= from {
                match found {
                    None => return None,
                    Some(f) if f >= p => return Some(f),
                    _ => {}
                }
            }
        }
        let bytes = hay.as_bytes();
        let mut i = p;
        let found = loop {
            let Some(off) = hay[i..].find('`') else {
                break None;
            };
            let start = i + off;
            let run = bytes[start..].iter().take_while(|b| **b == b'`').count();
            if run == n {
                break Some(start);
            }
            i = start + run;
        };
        self.0.insert(n, (p, found));
        found
    }
}

/// A code span starting at `pos` of `hay` (a backtick), as in CommonMark:
/// it opens with a run of n backticks and closes at the next run of
/// exactly n, so "` ```ui-js `" is the code "```ui-js". One space on each
/// side is stripped when both are there (that is how a span starting or
/// ending with a backtick is written). Returns the code (None when no
/// closing run follows), the opening run's length and the bytes used.
fn code_span<'a>(hay: &'a str, pos: usize, runs: &mut Runs) -> (Option<&'a str>, usize, usize) {
    let open = hay.as_bytes()[pos..]
        .iter()
        .take_while(|b| **b == b'`')
        .count();
    let body = pos + open;
    match runs.at(hay, open, body) {
        Some(start) => {
            let mut code = &hay[body..start];
            if code.len() >= 2
                && code.starts_with(' ')
                && code.ends_with(' ')
                && !code.trim().is_empty()
            {
                code = &code[1..code.len() - 1];
            }
            (Some(code), open, start + open - pos)
        }
        None => (None, open, open),
    }
}

/// Inline parser: ***bold italic***, **bold**, *italic*, `code`, [text](url).
/// Bold and italic nest (`**a *b* c**`).
pub fn parse_inline(input: &str) -> Vec<MdInline> {
    let mut out = Vec::new();
    let (mut star, mut tick, mut bracket) = (Next::new("*"), Next::new("`"), Next::new("["));
    let (mut double, mut triple) = (Next::new("**"), Next::new("***"));
    let (mut close_bracket, mut close_paren) = (Next::new("]"), Next::new(")"));
    let mut runs = Runs::default();
    let mut at = 0;
    while at < input.len() {
        // the earliest marker ("**" starts at a "*", so "*" covers it)
        let candidates = [
            star.at(input, at),
            tick.at(input, at),
            bracket.at(input, at),
        ];
        let Some(pos) = candidates.iter().flatten().min().copied() else {
            out.push(MdInline::Text(input[at..].to_string()));
            break;
        };
        let here = &input[pos..];
        let marker_len = if here.starts_with("**") { 2 } else { 1 };
        if pos > at {
            out.push(MdInline::Text(input[at..pos].to_string()));
        }
        let after = pos + marker_len;
        if here.starts_with("***") {
            if let Some(end) = triple.at(input, pos + 3) {
                if end > pos + 3 {
                    out.push(MdInline::Bold(vec![MdInline::Italic(parse_inline(
                        &input[pos + 3..end],
                    ))]));
                    at = end + 3;
                    continue;
                }
            }
        }
        if here.starts_with("**") {
            if let Some(end) = double.at(input, after) {
                out.push(MdInline::Bold(parse_inline(&input[after..end])));
                at = end + 2;
                continue;
            }
        } else if here.starts_with('*') {
            if let Some(end) = star.at(input, after) {
                if end > after {
                    out.push(MdInline::Italic(parse_inline(&input[after..end])));
                    at = end + 1;
                    continue;
                }
            }
        } else if here.starts_with('`') {
            let (code, len, used) = code_span(input, pos, &mut runs);
            match code {
                Some(code) => out.push(MdInline::Code(code.to_string())),
                // an unmatched backtick run is literal text, whole
                None => out.push(MdInline::Text(input[pos..pos + len].to_string())),
            }
            at = pos + used;
            continue;
        } else if here.starts_with('[') {
            if let Some(close) = close_bracket.at(input, after) {
                if input[close + 1..].starts_with('(') {
                    if let Some(end) = close_paren.at(input, close + 2) {
                        out.push(MdInline::Link {
                            text: input[after..close].to_string(),
                            url: input[close + 2..end].to_string(),
                        });
                        at = end + 1;
                        continue;
                    }
                }
            }
        }
        // no closing marker: emit the marker literally
        // (the text before it was pushed above)
        out.push(MdInline::Text(input[pos..pos + marker_len].to_string()));
        at = pos + marker_len;
    }
    // merge adjacent plain text
    let mut merged: Vec<MdInline> = Vec::new();
    for item in out {
        match (merged.last_mut(), &item) {
            (Some(MdInline::Text(a)), MdInline::Text(b)) => {
                a.push_str(b);
            }
            _ => merged.push(item),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_cached_matches_parse() {
        let text =
            "# Titel\n\nText mit **fett** und einer Tabelle:\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        assert_eq!(*super::parse_cached(text), super::parse(text));
        assert_eq!(*super::parse_cached(text), super::parse(text));
    }

    use super::*;

    #[test]
    fn headings_and_paragraph() {
        let blocks = parse("# Titel\n\nHallo Welt\n");
        assert_eq!(
            blocks[0],
            MdBlock::Heading(1, vec![MdInline::Text("Titel".into())])
        );
        assert_eq!(
            blocks[1],
            MdBlock::Para(vec![MdInline::Text("Hallo Welt".into())])
        );
    }

    #[test]
    fn code_fence_is_opaque() {
        let blocks =
            parse("vorher\n```rust\nfn main() { let x = 1; } // **not bold**\n```\nnachher\n");
        assert!(
            matches!(&blocks[1], MdBlock::Code { lang, code, closed: true } if lang == "rust" && code.contains("**not bold**"))
        );
        assert_eq!(
            blocks[2],
            MdBlock::Para(vec![MdInline::Text("nachher".into())])
        );
    }

    #[test]
    fn backticks_inside_a_line_are_no_fence() {
        // a real answer: an inline "` ```ui-js `" in a list item and in a
        // table cell used to open a code block that swallowed the rest
        let text = "- generative UI (` ```ui-js ` mit Sandbox)\n\n\
                    | Aspekt | Rust |\n|---|---|\n| **Sandboxing** | ` ```ui-js `-Blöcke laufen extern |\n\n\
                    Danach normaler Text.\n";
        let blocks = parse(text);
        assert!(
            !blocks.iter().any(|b| matches!(b, MdBlock::Code { .. })),
            "no code block: {blocks:?}"
        );
        assert!(matches!(&blocks[0], MdBlock::List(_)));
        assert!(matches!(&blocks[1], MdBlock::Table { rows, .. } if rows.len() == 1));
        assert_eq!(
            blocks[2],
            MdBlock::Para(vec![MdInline::Text("Danach normaler Text.".into())])
        );
    }

    #[test]
    fn fences_are_whole_lines() {
        // indented fence (inside a list item) opens; backticks after code
        // on a line do not close; a longer fence needs a long closer
        let blocks =
            parse("  ```rust\nlet s = \"```\";\n  ```\nweiter\n````md\n```\ninnen\n```\n````\n");
        assert!(
            matches!(&blocks[0], MdBlock::Code { lang, code, closed: true } if lang == "rust" && code == "let s = \"```\";")
        );
        assert_eq!(
            blocks[1],
            MdBlock::Para(vec![MdInline::Text("weiter".into())])
        );
        assert!(
            matches!(&blocks[2], MdBlock::Code { lang, code, closed: true } if lang == "md" && code == "```\ninnen\n```")
        );
        assert_eq!(blocks.len(), 3);
    }

    #[test]
    fn a_streaming_fence_waits_for_its_language_line() {
        let blocks = parse("text\n```ui-j");
        assert!(
            matches!(&blocks[1], MdBlock::Code { lang, code, closed: false } if lang.is_empty() && code.is_empty())
        );
    }

    #[test]
    fn unclosed_fence_is_marked_open() {
        let blocks = parse("text\n```ui-js\nfunction render(){");
        assert!(matches!(&blocks[1], MdBlock::Code { lang, closed: false, .. } if lang == "ui-js"));
    }

    #[test]
    fn code_spans_close_at_a_run_of_the_same_length() {
        assert_eq!(
            parse_inline("UI (` ```ui-js ` mit `boa_engine`)"),
            vec![
                MdInline::Text("UI (".into()),
                MdInline::Code("```ui-js".into()),
                MdInline::Text(" mit ".into()),
                MdInline::Code("boa_engine".into()),
                MdInline::Text(")".into()),
            ]
        );
        // double backticks hold a single one; spaces are kept inside
        assert_eq!(
            parse_inline("``a ` b``"),
            vec![MdInline::Code("a ` b".into())]
        );
        assert_eq!(parse_inline("` `"), vec![MdInline::Code(" ".into())]);
        // no closing run of that length: the backticks are text
        assert_eq!(
            parse_inline("ein `` offener `x`"),
            vec![
                MdInline::Text("ein `` offener ".into()),
                MdInline::Code("x".into()),
            ]
        );
    }

    #[test]
    fn inline_styles() {
        let inl = parse_inline(
            "Das ist **fett** und *kursiv* und `code` sowie [ein Link](https://example.com).",
        );
        assert!(inl.iter().any(|i| matches!(i, MdInline::Bold(_))));
        assert!(inl.iter().any(|i| matches!(i, MdInline::Italic(_))));
        assert!(inl
            .iter()
            .any(|i| matches!(i, MdInline::Code(c) if c == "code")));
        assert!(inl
            .iter()
            .any(|i| matches!(i, MdInline::Link { url, .. } if url == "https://example.com")));
    }

    #[test]
    fn bold_italic_and_nesting() {
        let inl = parse_inline("a ***beides*** b");
        assert_eq!(
            inl,
            vec![
                MdInline::Text("a ".into()),
                MdInline::Bold(vec![MdInline::Italic(vec![MdInline::Text(
                    "beides".into()
                )])]),
                MdInline::Text(" b".into()),
            ]
        );
        let inl = parse_inline("**fett *und kursiv* fett**");
        assert!(matches!(&inl[0], MdInline::Bold(inner)
            if inner.iter().any(|i| matches!(i, MdInline::Italic(_)))));
        // unclosed markers stay literal
        assert_eq!(
            parse_inline("2 *** 3"),
            vec![MdInline::Text("2 *** 3".into())]
        );
        assert_eq!(
            parse_inline("a * b und [x] y"),
            vec![MdInline::Text("a * b und [x] y".into())]
        );
    }

    #[test]
    fn list_and_quote() {
        let blocks = parse("- eins\n- zwei\n\n> Zitat\n");
        assert!(matches!(&blocks[0], MdBlock::List(items) if items.len() == 2));
        assert!(matches!(&blocks[1], MdBlock::Quote(_)));
    }

    #[test]
    fn table_parses_header_separator_and_rows() {
        let blocks = parse("Vorwort\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| **dick** | `x` |\n");
        assert_eq!(
            blocks,
            vec![
                MdBlock::Para(vec![MdInline::Text("Vorwort".into())]),
                MdBlock::Table {
                    columns: vec![parse_inline("A"), parse_inline("B")],
                    rows: vec![
                        vec![parse_inline("1"), parse_inline("2")],
                        vec![parse_inline("**dick**"), parse_inline("`x`")],
                    ],
                },
            ]
        );
    }

    #[test]
    fn table_needs_a_separator_row() {
        // without the |---| row the pipes stay plain text
        let blocks = parse("Vorher\n| a | b |\nNachher\n");
        assert_eq!(
            blocks,
            vec![MdBlock::Para(vec![
                MdInline::Text("Vorher".into()),
                MdInline::Text("\n".into()),
                MdInline::Text("| a | b |".into()),
                MdInline::Text("\n".into()),
                MdInline::Text("Nachher".into()),
            ])]
        );
        // wrong cell count in the separator is no separator either
        let blocks = parse("| a | b |\n| --- |\n");
        assert_eq!(
            blocks,
            vec![MdBlock::Para(vec![
                MdInline::Text("| a | b |".into()),
                MdInline::Text("\n".into()),
                MdInline::Text("| --- |".into()),
            ])]
        );
    }

    #[test]
    fn tables_split_surrounding_paragraphs() {
        // no blank lines around the table: still three blocks
        let blocks = parse("Intro\n| A | B |\n|---|---|\n| 1 | 2 |\nOutro");
        assert_eq!(
            blocks,
            vec![
                MdBlock::Para(vec![MdInline::Text("Intro".into())]),
                MdBlock::Table {
                    columns: vec![parse_inline("A"), parse_inline("B")],
                    rows: vec![vec![parse_inline("1"), parse_inline("2")]],
                },
                MdBlock::Para(vec![MdInline::Text("Outro".into())]),
            ]
        );
    }

    #[test]
    fn table_edge_cells() {
        // single column, empty cells, separator without surrounding pipes
        let blocks = parse("| A |\n|---|\n| 1 |\n|  |");
        assert_eq!(
            blocks,
            vec![MdBlock::Table {
                columns: vec![parse_inline("A")],
                rows: vec![vec![parse_inline("1")], vec![parse_inline("")]],
            }]
        );
        // header and separator can also stand without pipes at the ends
        assert_eq!(
            parse("A | B\n:-- | :-:\n"),
            vec![MdBlock::Table {
                columns: vec![parse_inline("A"), parse_inline("B")],
                rows: vec![],
            }]
        );
    }

    #[test]
    fn table_rows_fit_and_align_to_the_header() {
        // body rows are padded or truncated to the header width;
        // alignment colons are parsed (and ignored by the renderer)
        let blocks = parse("A | B\n:--- | ---:\n| 1 |\n1 | 2 | 3");
        assert_eq!(
            blocks,
            vec![MdBlock::Table {
                columns: vec![parse_inline("A"), parse_inline("B")],
                rows: vec![
                    vec![parse_inline("1"), parse_inline("")],
                    vec![parse_inline("1"), parse_inline("2")],
                ],
            }]
        );
    }

    #[test]
    fn table_escapes_and_block_ends() {
        // `\|` stays inside one cell
        let blocks = parse("| A | B |\n|---|---|\n| x \\| y | z |");
        assert_eq!(
            blocks,
            vec![MdBlock::Table {
                columns: vec![parse_inline("A"), parse_inline("B")],
                rows: vec![vec![parse_inline("x | y"), parse_inline("z")]],
            }]
        );
        // a list (or any other block) ends a table instead of becoming a row
        let blocks = parse("| A | B |\n|---|---|\n- a | b");
        assert_eq!(
            blocks,
            vec![
                MdBlock::Table {
                    columns: vec![parse_inline("A"), parse_inline("B")],
                    rows: vec![],
                },
                MdBlock::List(vec![parse_inline("a | b")]),
            ]
        );
        // a heading line never starts a table, even with pipes
        let blocks = parse("# Kein | Header\n|---|---|");
        assert_eq!(
            blocks,
            vec![
                MdBlock::Heading(1, parse_inline("Kein | Header")),
                MdBlock::Para(vec![MdInline::Text("|---|---|".into())]),
            ]
        );
    }
}

/// The quadratic inline parser before audit M5, kept as the reference the
/// linear one must match exactly.
#[cfg(test)]
mod inline_reference {
    use super::MdInline;

    /// A code span at the start of `s` (which begins with a backtick), as in
    /// CommonMark: it opens with a run of n backticks and closes at the next run
    /// of exactly n, so "` ```ui-js `" is the code "```ui-js". One space on
    /// each side is stripped when both are there (that is how a span starting
    /// or ending with a backtick is written). Returns the code (None when no
    /// closing run follows), the opening run's length and the bytes consumed.
    fn ref_code_span(s: &str) -> (Option<&str>, usize, usize) {
        let open = s.bytes().take_while(|b| *b == b'`').count();
        let body = &s[open..];
        let mut i = 0;
        while let Some(off) = body[i..].find('`') {
            let start = i + off;
            let run = body[start..].bytes().take_while(|b| *b == b'`').count();
            if run == open {
                let mut code = &body[..start];
                if code.len() >= 2
                    && code.starts_with(' ')
                    && code.ends_with(' ')
                    && !code.trim().is_empty()
                {
                    code = &code[1..code.len() - 1];
                }
                return (Some(code), open, open + start + run);
            }
            i = start + run;
        }
        (None, open, open)
    }

    /// Inline parser: ***bold italic***, **bold**, *italic*, `code`, [text](url).
    /// Bold and italic nest (`**a *b* c**`).
    fn ref_parse_inline(input: &str) -> Vec<MdInline> {
        let mut out = Vec::new();
        let mut rest = input;
        while !rest.is_empty() {
            // find earliest marker
            let candidates = [
                rest.find("**"),
                rest.find('*'),
                rest.find('`'),
                rest.find('['),
            ];
            let Some(pos) = candidates.iter().flatten().min().copied() else {
                out.push(MdInline::Text(rest.to_string()));
                break;
            };
            let marker_len = if rest[pos..].starts_with("**") { 2 } else { 1 };
            if pos > 0 {
                out.push(MdInline::Text(rest[..pos].to_string()));
            }
            let after = &rest[pos + marker_len..];
            if rest[pos..].starts_with("***") {
                let inner_start = &rest[pos + 3..];
                if let Some(end) = inner_start.find("***") {
                    if end > 0 {
                        out.push(MdInline::Bold(vec![MdInline::Italic(ref_parse_inline(
                            &inner_start[..end],
                        ))]));
                        rest = &inner_start[end + 3..];
                        continue;
                    }
                }
            }
            if rest[pos..].starts_with("**") {
                if let Some(end) = after.find("**") {
                    out.push(MdInline::Bold(ref_parse_inline(&after[..end])));
                    rest = &after[end + 2..];
                    continue;
                }
            } else if rest[pos..].starts_with('*') {
                if let Some(end) = after.find('*') {
                    if end > 0 {
                        out.push(MdInline::Italic(ref_parse_inline(&after[..end])));
                        rest = &after[end + 1..];
                        continue;
                    }
                }
            } else if rest[pos..].starts_with('`') {
                let (code, len, used) = ref_code_span(&rest[pos..]);
                match code {
                    Some(code) => out.push(MdInline::Code(code.to_string())),
                    // an unmatched backtick run is literal text, whole
                    None => out.push(MdInline::Text(rest[pos..pos + len].to_string())),
                }
                rest = &rest[pos + used..];
                continue;
            } else if rest[pos..].starts_with('[') {
                if let Some(close) = after.find(']') {
                    let text = &after[..close];
                    let after_close = &after[close + 1..];
                    if let Some(url) = after_close.strip_prefix('(') {
                        if let Some(end) = url.find(')') {
                            out.push(MdInline::Link {
                                text: text.to_string(),
                                url: url[..end].to_string(),
                            });
                            rest = &url[end + 1..];
                            continue;
                        }
                    }
                }
            }
            // no closing marker: emit the marker literally
            // (the text before it was pushed above)
            out.push(MdInline::Text(rest[pos..pos + marker_len].to_string()));
            rest = &rest[pos + marker_len..];
        }
        let mut merged: Vec<MdInline> = Vec::new();
        for item in out {
            match (merged.last_mut(), &item) {
                (Some(MdInline::Text(a)), MdInline::Text(b)) => a.push_str(b),
                _ => merged.push(item),
            }
        }
        merged
    }

    /// xorshift, so the cases are the same on every run
    fn next(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    #[test]
    fn the_linear_parser_matches_the_reference() {
        const ALPHABET: [&str; 10] = ["*", "*", "`", "`", "[", "]", "(", ")", "a", " ü"];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..60_000 {
            let len = (next(&mut state) % 24) as usize;
            let input: String = (0..len)
                .map(|_| ALPHABET[(next(&mut state) % ALPHABET.len() as u64) as usize])
                .collect();
            assert_eq!(
                super::parse_inline(&input),
                ref_parse_inline(&input),
                "input {input:?}"
            );
        }
    }

    /// Worst cases of the old parser stay fast: 256 KiB lines of a single
    /// marker, or of unmatched backtick runs of growing length.
    #[test]
    fn marker_floods_parse_in_linear_time() {
        let runs: String = (1..700).map(|n| "`".repeat(n) + " ").collect();
        for flood in [
            "[".repeat(1 << 18),
            "*a".repeat(1 << 17),
            "**x".repeat(1 << 16),
            runs,
        ] {
            let t = std::time::Instant::now();
            let parsed = super::parse_inline(&flood);
            assert!(!parsed.is_empty());
            // generous for debug builds; the old parser took minutes here
            assert!(
                t.elapsed() < std::time::Duration::from_secs(2),
                "{:?}",
                t.elapsed()
            );
        }
    }
}
