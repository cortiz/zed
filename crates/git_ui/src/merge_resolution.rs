//! Line-level three-way merge used to auto-resolve "simple" conflicts.
//!
//! Git reports a conflict whenever both sides touch the same or *adjacent*
//! lines. Many of those conflicts are mechanically resolvable: one side edits
//! line 2 and the other edits line 3, both sides make the identical change,
//! or one side didn't change the region at all. This module re-merges a single
//! conflict region line by line and returns the merged text when no two
//! changes overlap.

use std::ops::Range;

/// A change one side made to the base, expressed in base line indices.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Change<'a> {
    base_rows: Range<usize>,
    new_lines: &'a [&'a str],
}

/// Merges `ours` and `theirs` against their common ancestor `base`.
///
/// Returns `None` when the two sides make overlapping, non-identical changes,
/// or when both insert different text at the same position — the cases that
/// genuinely need a human. Changes that merely touch (one ends on the line
/// where the other begins) are combined in base order.
pub(crate) fn merge_non_overlapping(base: &str, ours: &str, theirs: &str) -> Option<String> {
    if ours == theirs || theirs == base {
        return Some(ours.to_owned());
    }
    if ours == base {
        return Some(theirs.to_owned());
    }

    let base_lines: Vec<&str> = base.split_inclusive('\n').collect();
    let ours_lines: Vec<&str> = ours.split_inclusive('\n').collect();
    let theirs_lines: Vec<&str> = theirs.split_inclusive('\n').collect();
    let ours_changes = changes(base, ours, &ours_lines);
    let theirs_changes = changes(base, theirs, &theirs_lines);

    let mut merged = String::with_capacity(ours.len().max(theirs.len()));
    let mut base_row = 0;
    let mut ours_changes = ours_changes.into_iter().peekable();
    let mut theirs_changes = theirs_changes.into_iter().peekable();

    loop {
        let next = match (ours_changes.peek(), theirs_changes.peek()) {
            (None, None) => break,
            (Some(_), None) => ours_changes.next()?,
            (None, Some(_)) => theirs_changes.next()?,
            (Some(ours), Some(theirs)) => {
                if ours == theirs {
                    theirs_changes.next();
                    ours_changes.next()?
                } else if conflicts(&ours.base_rows, &theirs.base_rows) {
                    return None;
                } else if precedes(&ours.base_rows, &theirs.base_rows) {
                    ours_changes.next()?
                } else {
                    theirs_changes.next()?
                }
            }
        };

        // Changes are applied in base order and never overlap, so the next
        // one always starts at or after the current position.
        debug_assert!(next.base_rows.start >= base_row);
        for line in &base_lines[base_row..next.base_rows.start] {
            merged.push_str(line);
        }
        for line in next.new_lines {
            merged.push_str(line);
        }
        base_row = next.base_rows.end;
    }

    for line in &base_lines[base_row..] {
        merged.push_str(line);
    }
    Some(merged)
}

fn changes<'a>(base: &str, new: &str, new_lines: &'a [&'a str]) -> Vec<Change<'a>> {
    language::line_diff(base, new)
        .into_iter()
        .map(|(base_rows, new_rows)| Change {
            base_rows: base_rows.start as usize..base_rows.end as usize,
            new_lines: &new_lines[new_rows.start as usize..new_rows.end as usize],
        })
        .collect()
}

/// Two changes conflict when their base ranges overlap, or when both are
/// insertions at the same position (their relative order would be a guess).
fn conflicts(a: &Range<usize>, b: &Range<usize>) -> bool {
    let both_insert_at_same_row = a.is_empty() && b.is_empty() && a.start == b.start;
    let overlap = a.start < b.end && b.start < a.end;
    both_insert_at_same_row || overlap
}

/// Orders two non-conflicting changes. An insertion at the row where a
/// replacement starts goes before that replacement.
fn precedes(a: &Range<usize>, b: &Range<usize>) -> bool {
    (a.start, a.end) <= (b.start, b.end)
}

#[cfg(test)]
mod tests {
    use super::merge_non_overlapping;

    #[test]
    fn test_one_side_unchanged() {
        assert_eq!(
            merge_non_overlapping("a\n", "a\n", "b\n").as_deref(),
            Some("b\n")
        );
        assert_eq!(
            merge_non_overlapping("a\n", "b\n", "a\n").as_deref(),
            Some("b\n")
        );
    }

    #[test]
    fn test_identical_changes() {
        assert_eq!(
            merge_non_overlapping("a\nb\n", "a\nX\n", "a\nX\n").as_deref(),
            Some("a\nX\n")
        );
    }

    #[test]
    fn test_adjacent_line_edits_merge() {
        // Git conflicts on these because the edited lines touch.
        assert_eq!(
            merge_non_overlapping(
                "one\ntwo\nthree\n",
                "ONE\ntwo\nthree\n",
                "one\nTWO\nthree\n"
            )
            .as_deref(),
            Some("ONE\nTWO\nthree\n")
        );
    }

    #[test]
    fn test_insertion_next_to_edit() {
        assert_eq!(
            merge_non_overlapping("a\nb\n", "a\nb\nours\n", "a\nB\n").as_deref(),
            Some("a\nB\nours\n")
        );
        assert_eq!(
            merge_non_overlapping("a\nb\n", "first\na\nb\n", "A\nb\n").as_deref(),
            Some("first\nA\nb\n")
        );
    }

    #[test]
    fn test_overlapping_edits_are_left_alone() {
        assert_eq!(merge_non_overlapping("a\n", "b\n", "c\n"), None);
        assert_eq!(
            merge_non_overlapping("a\nb\nc\n", "a\nX\nY\n", "a\nb\nZ\n"),
            None
        );
    }

    #[test]
    fn test_different_insertions_at_same_row_are_left_alone() {
        assert_eq!(merge_non_overlapping("a\n", "a\nb\n", "a\nc\n"), None);
    }

    #[test]
    fn test_empty_sides() {
        // One side deleted the region, the other left it untouched.
        assert_eq!(merge_non_overlapping("a\n", "", "a\n").as_deref(), Some(""));
        // Both sides deleted different adjacent lines.
        assert_eq!(
            merge_non_overlapping("a\nb\n", "b\n", "a\n").as_deref(),
            Some("")
        );
    }

    #[test]
    fn test_missing_trailing_newline() {
        assert_eq!(
            merge_non_overlapping("a\nb", "A\nb", "a\nB").as_deref(),
            Some("A\nB")
        );
    }
}
