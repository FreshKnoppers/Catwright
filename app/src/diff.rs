//! Line diff for tool-call and permission previews: a longest-common-
//! subsequence diff of two texts, folded to hunks with context. Pure; Vibe
//! reports whole files for `write_file` and fragments for `edit`, so there
//! are no absolute line numbers here.

/// How one line of a diff differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Same,
    Added,
    Removed,
}

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

/// One visible piece of a folded diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Line(DiffLine),
    /// `n` unchanged lines are folded away here.
    Skipped(usize),
}

/// Above this many table cells the changed middle is shown as a block of
/// removed lines plus a block of added lines (the table would cost too much
/// memory; the prefix and suffix have already been trimmed).
const MAX_LCS_CELLS: usize = 4_000_000;

/// Unchanged context lines kept around a change.
pub const CONTEXT: usize = 3;

/// The diff of `old` (`None` = new file) against `new`, line by line.
pub fn line_diff(old: Option<&str>, new: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old.map(|o| o.lines().collect()).unwrap_or_default();
    let new_lines: Vec<&str> = new.lines().collect();
    let common_prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let common_suffix = old_lines[common_prefix..]
        .iter()
        .rev()
        .zip(new_lines[common_prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_mid = &old_lines[common_prefix..old_lines.len() - common_suffix];
    let new_mid = &new_lines[common_prefix..new_lines.len() - common_suffix];

    let mut out = Vec::with_capacity(old_lines.len().max(new_lines.len()) + 8);
    let line = |kind, text: &str| DiffLine {
        kind,
        text: text.to_string(),
    };
    out.extend(
        old_lines[..common_prefix]
            .iter()
            .map(|l| line(LineKind::Same, l)),
    );
    let cells = (old_mid.len() + 1).saturating_mul(new_mid.len() + 1);
    if cells > MAX_LCS_CELLS {
        out.extend(old_mid.iter().map(|l| line(LineKind::Removed, l)));
        out.extend(new_mid.iter().map(|l| line(LineKind::Added, l)));
    } else {
        lcs_middle(old_mid, new_mid, &mut out);
    }
    out.extend(
        old_lines[old_lines.len() - common_suffix..]
            .iter()
            .map(|l| line(LineKind::Same, l)),
    );
    out
}

/// LCS table walk: removals come before additions within a change.
fn lcs_middle(old: &[&str], new: &[&str], out: &mut Vec<DiffLine>) {
    let (n, m) = (old.len(), new.len());
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && old[i] == new[j] {
            out.push(DiffLine {
                kind: LineKind::Same,
                text: old[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if i < n && (j == m || table[(i + 1) * width + j] >= table[i * width + j + 1]) {
            out.push(DiffLine {
                kind: LineKind::Removed,
                text: old[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: LineKind::Added,
                text: new[j].to_string(),
            });
            j += 1;
        }
    }
}

/// `(added, removed)` lines of a diff.
#[cfg(test)]
pub fn counts(lines: &[DiffLine]) -> (usize, usize) {
    lines.iter().fold((0, 0), |(a, r), l| match l.kind {
        LineKind::Added => (a + 1, r),
        LineKind::Removed => (a, r + 1),
        LineKind::Same => (a, r),
    })
}

/// Fold long unchanged runs, keeping `context` lines next to each change.
/// Open diffs redraw every frame; their folded segments do not change.
/// Cached by (old, new) hash.
pub fn folded_cached(old: Option<&str>, new: &str) -> std::sync::Arc<Vec<Segment>> {
    static CACHE: std::sync::OnceLock<crate::memo::Memo<Vec<Segment>>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| crate::memo::Memo::new(512))
        .get_or_insert(crate::memo::hash_pair(old, new), || {
            fold(&line_diff(old, new), CONTEXT)
        })
}

pub fn fold(lines: &[DiffLine], context: usize) -> Vec<Segment> {
    let mut keep = vec![false; lines.len()];
    for (i, l) in lines.iter().enumerate() {
        if l.kind != LineKind::Same {
            let from = i.saturating_sub(context);
            let to = (i + context + 1).min(lines.len());
            keep[from..to].fill(true);
        }
    }
    let mut out = Vec::new();
    let mut skipped = 0;
    for (i, l) in lines.iter().enumerate() {
        if keep[i] {
            if skipped > 0 {
                out.push(Segment::Skipped(skipped));
                skipped = 0;
            }
            out.push(Segment::Line(l.clone()));
        } else {
            skipped += 1;
        }
    }
    if skipped > 0 {
        out.push(Segment::Skipped(skipped));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn folded_cached_matches_the_direct_computation() {
        let old = "a\nb\nc\nd\ne";
        let new = "a\nx\nc\ne";
        let direct = super::fold(&super::line_diff(Some(old), new), super::CONTEXT);
        assert_eq!(*super::folded_cached(Some(old), new), direct);
        assert_eq!(
            *super::folded_cached(None, new),
            *super::folded_cached(None, new)
        );
    }

    use super::*;

    fn kinds(lines: &[DiffLine]) -> Vec<(LineKind, &str)> {
        lines.iter().map(|l| (l.kind, l.text.as_str())).collect()
    }

    #[test]
    fn new_file_is_all_added() {
        let d = line_diff(None, "a\nb\n");
        assert_eq!(
            kinds(&d),
            vec![(LineKind::Added, "a"), (LineKind::Added, "b")]
        );
        assert_eq!(counts(&d), (2, 0));
    }

    #[test]
    fn identical_texts_have_no_changes() {
        let d = line_diff(Some("a\nb"), "a\nb");
        assert_eq!(counts(&d), (0, 0));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn single_line_edit_is_one_removal_and_one_addition() {
        let old = (0..50).map(|i| format!("z{i}\n")).collect::<String>();
        let new = old.replace("z25\n", "ZZ\n");
        let d = line_diff(Some(&old), &new);
        assert_eq!(counts(&d), (1, 1));
        assert_eq!(d.iter().filter(|l| l.kind == LineKind::Same).count(), 49);
    }

    #[test]
    fn insertion_and_deletion_keep_the_common_lines() {
        let d = line_diff(Some("a\nb\nc\nd"), "a\nx\nc\ne\nd");
        assert_eq!(
            kinds(&d),
            vec![
                (LineKind::Same, "a"),
                (LineKind::Removed, "b"),
                (LineKind::Added, "x"),
                (LineKind::Same, "c"),
                (LineKind::Added, "e"),
                (LineKind::Same, "d"),
            ]
        );
    }

    #[test]
    fn replaced_block_counts_every_line() {
        let d = line_diff(Some("x\none\ntwo\ny"), "x\nthree\ny");
        assert_eq!(counts(&d), (1, 2));
    }

    #[test]
    fn oversized_middle_falls_back_to_block_replace() {
        let old: String = (0..3000).map(|i| format!("a{i}\n")).collect();
        let new: String = (0..3000).map(|i| format!("b{i}\n")).collect();
        let d = line_diff(Some(&old), &new);
        assert_eq!(counts(&d), (3000, 3000));
    }

    #[test]
    fn fold_keeps_context_around_changes() {
        let old: String = (0..40).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l20\n", "changed\n");
        let d = line_diff(Some(&old), &new);
        let f = fold(&d, 2);
        assert_eq!(f[0], Segment::Skipped(18));
        assert!(matches!(f.last(), Some(Segment::Skipped(17))));
        let shown = f.iter().filter(|s| matches!(s, Segment::Line(_))).count();
        assert_eq!(
            shown,
            2 + 2 + 2,
            "two context lines each side plus the change"
        );
    }

    #[test]
    fn fold_of_a_new_file_shows_everything() {
        let d = line_diff(None, "a\nb\nc");
        assert_eq!(fold(&d, 3).len(), 3);
    }
}
