//! Parse a unified-diff text into a `DiffPositionIndex` that maps each rendered
//! line back to its source coordinates.
//!
//! "Hunk position" follows GitHub's review-comment positioning:
//! 1-based index within a file's diff body, incremented on every `+`/`-`/context
//! line. Hunk headers (`@@ ... @@`) and `\ No newline at end of file` markers do
//! **not** advance the position counter and are themselves un-positioned.
//!
//! Used by the diff viewer to render unified diffs and by the line-comment
//! composer to translate a focused screen line into the `(side, file_line,
//! hunk_position)` triple GitHub expects.

use crate::data::models::DiffSide;

/// Source coordinates for one rendered line of a unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPosition {
    /// File path the line belongs to. For pre-hunk file headers this is the
    /// path of the file the header introduces.
    pub path: String,
    /// `None` for file headers, hunk headers, and `\ No newline` markers.
    pub side_and_line: Option<(DiffSide, u32)>,
    /// 1-based GitHub review-comment position, or `None` for headers and
    /// `\ No newline` markers.
    pub hunk_position: Option<u32>,
}

/// One `DiffPosition` per rendered line of the source diff, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffPositionIndex {
    pub lines: Vec<DiffPosition>,
}

impl DiffPositionIndex {
    /// Index of the next line that begins a new hunk (i.e. `hunk_position == Some(1)`),
    /// strictly after `from`. Returns `None` if none exists.
    #[must_use]
    pub fn next_hunk_after(&self, from: usize) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .skip(from.saturating_add(1))
            .find_map(|(i, p)| (p.hunk_position == Some(1)).then_some(i))
    }

    /// Index of the previous line that begins a new hunk, strictly before `from`.
    #[must_use]
    pub fn prev_hunk_before(&self, from: usize) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .take(from)
            .rev()
            .find_map(|(i, p)| (p.hunk_position == Some(1)).then_some(i))
    }

    /// Translate an inclusive rendered-line range `[start_idx, end_idx]` into a
    /// `NewLineComment` skeleton (body left empty for the composer to fill).
    ///
    /// The first positioned line at or after `start_idx` defines `start_line` /
    /// `start_side`; the last positioned line at or before `end_idx` defines
    /// `line` / `side`. If both endpoints collapse onto the same source line,
    /// `start_line` and `start_side` are `None` (GitHub's single-line form).
    /// Returns `None` if no positioned line exists in the range or if the
    /// endpoints span different files.
    #[must_use]
    pub fn build_line_comment(
        &self,
        start_idx: usize,
        end_idx: usize,
    ) -> Option<crate::data::github::NewLineComment> {
        let (lo, hi) = if start_idx <= end_idx {
            (start_idx, end_idx)
        } else {
            (end_idx, start_idx)
        };
        let first = self
            .lines
            .iter()
            .enumerate()
            .skip(lo)
            .take(hi.saturating_sub(lo).saturating_add(1))
            .find(|(_, p)| p.side_and_line.is_some())?;
        let last = self
            .lines
            .iter()
            .enumerate()
            .take(hi.saturating_add(1))
            .rev()
            .take(hi.saturating_sub(lo).saturating_add(1))
            .find(|(_, p)| p.side_and_line.is_some())?;

        if first.1.path != last.1.path {
            return None;
        }

        let (first_side, first_line) = first.1.side_and_line?;
        let (last_side, last_line) = last.1.side_and_line?;

        let single = first.0 == last.0;
        Some(crate::data::github::NewLineComment {
            path: last.1.path.clone(),
            side: last_side,
            line: last_line,
            start_side: if single { None } else { Some(first_side) },
            start_line: if single { None } else { Some(first_line) },
            body: String::new(),
        })
    }
}

impl DiffPositionIndex {
    /// Parse a unified diff (as produced by `git diff` or GitHub's
    /// `.diff` endpoint) into a position index.
    ///
    /// The input is the raw diff text including `diff --git`, `index`,
    /// `---`/`+++`, and `@@` header lines.
    #[must_use]
    pub fn parse(diff: &str) -> Self {
        let mut lines = Vec::new();
        let mut current_path: Option<String> = None;
        let mut left_line: u32 = 0;
        let mut right_line: u32 = 0;
        let mut hunk_position: u32 = 0;
        let mut in_hunk = false;

        for raw in diff.split_inclusive('\n') {
            let line = raw.strip_suffix('\n').unwrap_or(raw);

            if let Some(rest) = line.strip_prefix("diff --git ") {
                // `diff --git a/<path> b/<path>` — take the b-side path.
                current_path = parse_diff_git_path(rest);
                in_hunk = false;
                lines.push(DiffPosition {
                    path: current_path.clone().unwrap_or_default(),
                    side_and_line: None,
                    hunk_position: None,
                });
                continue;
            }

            // `+++ b/<path>` overrides the path (handles `/dev/null` and renames).
            if let Some(rest) = line.strip_prefix("+++ ") {
                if let Some(p) = parse_marker_path(rest) {
                    current_path = Some(p);
                }
                in_hunk = false;
                lines.push(DiffPosition {
                    path: current_path.clone().unwrap_or_default(),
                    side_and_line: None,
                    hunk_position: None,
                });
                continue;
            }

            if let Some(rest) = line.strip_prefix("@@ ") {
                if let Some((left_start, right_start)) = parse_hunk_header(rest) {
                    left_line = left_start;
                    right_line = right_start;
                    hunk_position = 0;
                    in_hunk = true;
                }
                lines.push(DiffPosition {
                    path: current_path.clone().unwrap_or_default(),
                    side_and_line: None,
                    hunk_position: None,
                });
                continue;
            }

            if !in_hunk {
                // File-header noise (`index`, `---`, `similarity`, ...).
                lines.push(DiffPosition {
                    path: current_path.clone().unwrap_or_default(),
                    side_and_line: None,
                    hunk_position: None,
                });
                continue;
            }

            if line.starts_with('\\') {
                // `\ No newline at end of file` — un-positioned, doesn't advance counters.
                lines.push(DiffPosition {
                    path: current_path.clone().unwrap_or_default(),
                    side_and_line: None,
                    hunk_position: None,
                });
                continue;
            }

            if let Some(first) = line.chars().next() {
                let entry = consume_body_line(
                    first,
                    current_path.as_deref().unwrap_or_default(),
                    &mut left_line,
                    &mut right_line,
                    &mut hunk_position,
                );
                lines.push(entry);
            }
        }

        Self { lines }
    }
}

fn consume_body_line(
    first: char,
    path: &str,
    left_line: &mut u32,
    right_line: &mut u32,
    hunk_position: &mut u32,
) -> DiffPosition {
    match first {
        '+' => {
            *hunk_position += 1;
            let pos = DiffPosition {
                path: path.to_string(),
                side_and_line: Some((DiffSide::Right, *right_line)),
                hunk_position: Some(*hunk_position),
            };
            *right_line += 1;
            pos
        }
        '-' => {
            *hunk_position += 1;
            let pos = DiffPosition {
                path: path.to_string(),
                side_and_line: Some((DiffSide::Left, *left_line)),
                hunk_position: Some(*hunk_position),
            };
            *left_line += 1;
            pos
        }
        ' ' => {
            *hunk_position += 1;
            let pos = DiffPosition {
                path: path.to_string(),
                side_and_line: Some((DiffSide::Right, *right_line)),
                hunk_position: Some(*hunk_position),
            };
            *left_line += 1;
            *right_line += 1;
            pos
        }
        _ => DiffPosition {
            path: path.to_string(),
            side_and_line: None,
            hunk_position: None,
        },
    }
}

fn parse_diff_git_path(rest: &str) -> Option<String> {
    // `a/path b/path` — split on the first ` b/`.
    let idx = rest.find(" b/")?;
    Some(rest[idx + 3..].to_string())
}

fn parse_marker_path(rest: &str) -> Option<String> {
    let trimmed = rest.trim();
    if trimmed == "/dev/null" {
        return None;
    }
    trimmed.strip_prefix("b/").map(str::to_string)
}

/// Parse `-L,C +R,C @@ ...` and return `(left_start, right_start)`.
fn parse_hunk_header(rest: &str) -> Option<(u32, u32)> {
    let end = rest.find(" @@")?;
    let header = &rest[..end];
    let mut parts = header.split_whitespace();
    let minus = parts.next()?;
    let plus = parts.next()?;
    let left_start = minus.strip_prefix('-')?.split(',').next()?.parse().ok()?;
    let right_start = plus.strip_prefix('+')?.split(',').next()?.parse().ok()?;
    Some((left_start, right_start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_file_diff_with_added_removed_context_multi_hunk_and_no_newline() {
        let diff = "\
diff --git a/file_a.rs b/file_a.rs
index 1111111..2222222 100644
--- a/file_a.rs
+++ b/file_a.rs
@@ -1,3 +1,4 @@
 ctx1
-removed_a
+added_a1
+added_a2
 ctx2
@@ -10,2 +11,2 @@
 ctx3
-old
+new
diff --git a/file_b.rs b/file_b.rs
index 3333333..4444444 100644
--- a/file_b.rs
+++ b/file_b.rs
@@ -1,1 +1,1 @@
-only_b_removed
+only_b_added
\\ No newline at end of file
diff --git a/file_c.rs b/file_c.rs
new file mode 100644
index 0000000..5555555
--- /dev/null
+++ b/file_c.rs
@@ -0,0 +1,2 @@
+brand_new_1
+brand_new_2
";
        let index = DiffPositionIndex::parse(diff);

        // Collect positioned lines (side/file_line/hunk_position) per path.
        let positioned: Vec<(&str, DiffSide, u32, u32)> = index
            .lines
            .iter()
            .filter_map(|p| {
                let (side, file_line) = p.side_and_line?;
                let hp = p.hunk_position?;
                Some((p.path.as_str(), side, file_line, hp))
            })
            .collect();

        // file_a.rs hunk 1 (left starts at 1, right starts at 1):
        // ctx1  → L=1/R=1 hp=1   (rendered as Right)
        // -removed_a → L=2       hp=2
        // +added_a1  → R=2       hp=3
        // +added_a2  → R=3       hp=4
        // ctx2  → L=3/R=4        hp=5
        //
        // file_a.rs hunk 2 (left starts at 10, right starts at 11; hp resets):
        // ctx3  → L=10/R=11      hp=1
        // -old  → L=11           hp=2
        // +new  → R=12           hp=3
        assert_eq!(
            positioned,
            vec![
                ("file_a.rs", DiffSide::Right, 1, 1),
                ("file_a.rs", DiffSide::Left, 2, 2),
                ("file_a.rs", DiffSide::Right, 2, 3),
                ("file_a.rs", DiffSide::Right, 3, 4),
                ("file_a.rs", DiffSide::Right, 4, 5),
                ("file_a.rs", DiffSide::Right, 11, 1),
                ("file_a.rs", DiffSide::Left, 11, 2),
                ("file_a.rs", DiffSide::Right, 12, 3),
                ("file_b.rs", DiffSide::Left, 1, 1),
                ("file_b.rs", DiffSide::Right, 1, 2),
                ("file_c.rs", DiffSide::Right, 1, 1),
                ("file_c.rs", DiffSide::Right, 2, 2),
            ]
        );

        // Every input line is represented exactly once in the index.
        assert_eq!(index.lines.len(), diff.lines().count());

        // The "\\ No newline" line and all header lines are un-positioned.
        let unpositioned = index
            .lines
            .iter()
            .filter(|p| p.side_and_line.is_none())
            .count();
        assert_eq!(unpositioned, index.lines.len() - positioned.len());
    }

    #[test]
    fn hunk_navigation_finds_next_and_prev_hunk_starts() {
        let diff = "\
diff --git a/x.rs b/x.rs
--- a/x.rs
+++ b/x.rs
@@ -1,2 +1,2 @@
 a
-b
+B
@@ -10,2 +10,2 @@
 c
-d
+D
@@ -20,1 +20,1 @@
-e
+E
";
        let index = DiffPositionIndex::parse(diff);
        let hunk_starts: Vec<usize> = index
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, p)| (p.hunk_position == Some(1)).then_some(i))
            .collect();
        assert_eq!(hunk_starts.len(), 3);

        assert_eq!(index.next_hunk_after(0), Some(hunk_starts[0]));
        assert_eq!(index.next_hunk_after(hunk_starts[0]), Some(hunk_starts[1]));
        assert_eq!(index.next_hunk_after(hunk_starts[1]), Some(hunk_starts[2]));
        assert_eq!(index.next_hunk_after(hunk_starts[2]), None);

        assert_eq!(index.prev_hunk_before(hunk_starts[2]), Some(hunk_starts[1]));
        assert_eq!(index.prev_hunk_before(hunk_starts[1]), Some(hunk_starts[0]));
        assert_eq!(index.prev_hunk_before(hunk_starts[0]), None);
    }

    fn small_diff() -> &'static str {
        "\
diff --git a/x.rs b/x.rs
index 1111111..2222222 100644
--- a/x.rs
+++ b/x.rs
@@ -1,3 +1,4 @@
 fn main() {
     let x = 1;
+    let y = 2;
 }
"
    }

    #[test]
    fn build_line_comment_single_line_added_omits_start() {
        let idx = DiffPositionIndex::parse(small_diff());
        let added_idx = idx
            .lines
            .iter()
            .position(|p| matches!(p.side_and_line, Some((DiffSide::Right, 3))))
            .expect("the +let y line");

        let c = idx
            .build_line_comment(added_idx, added_idx)
            .expect("single-line comment built");

        assert_eq!(c.path, "x.rs");
        assert_eq!(c.side, DiffSide::Right);
        assert_eq!(c.line, 3);
        assert!(
            c.start_line.is_none(),
            "single-line: start_line must be None"
        );
        assert!(
            c.start_side.is_none(),
            "single-line: start_side must be None"
        );
        assert!(c.body.is_empty(), "body left empty for composer");
    }

    #[test]
    fn build_line_comment_multi_line_range_sets_start_and_end() {
        let idx = DiffPositionIndex::parse(small_diff());
        let ctx_idx = idx
            .lines
            .iter()
            .position(|p| matches!(p.side_and_line, Some((DiffSide::Right, 2))))
            .expect("the let x = 1 line");
        let added_idx = idx
            .lines
            .iter()
            .position(|p| matches!(p.side_and_line, Some((DiffSide::Right, 3))))
            .expect("the +let y line");

        let c = idx
            .build_line_comment(ctx_idx, added_idx)
            .expect("multi-line comment built");

        assert_eq!(c.path, "x.rs");
        assert_eq!(c.start_side, Some(DiffSide::Right));
        assert_eq!(c.start_line, Some(2));
        assert_eq!(c.side, DiffSide::Right);
        assert_eq!(c.line, 3);
    }

    #[test]
    fn build_line_comment_range_starting_on_header_skips_to_first_positioned() {
        let idx = DiffPositionIndex::parse(small_diff());
        let hunk_header_idx = idx
            .lines
            .iter()
            .position(|p| p.hunk_position.is_none() && p.path == "x.rs")
            .expect("a header line for x.rs");
        let added_idx = idx
            .lines
            .iter()
            .position(|p| matches!(p.side_and_line, Some((DiffSide::Right, 3))))
            .expect("+let y line");

        let c = idx
            .build_line_comment(hunk_header_idx, added_idx)
            .expect("header-skipping comment built");

        assert_eq!(c.path, "x.rs");
        assert_eq!(c.side, DiffSide::Right);
        assert_eq!(c.line, 3);
        assert!(c.start_line.is_some(), "range covers >1 positioned line");
    }

    #[test]
    fn build_line_comment_range_with_no_positioned_lines_returns_none() {
        let idx = DiffPositionIndex::parse(small_diff());
        let first_header = 0;
        let last_header = idx
            .lines
            .iter()
            .position(|p| p.hunk_position == Some(1))
            .expect("first body line")
            - 1;
        assert!(idx.build_line_comment(first_header, last_header).is_none());
    }

    #[test]
    fn build_line_comment_left_side_deletion() {
        let diff = "\
diff --git a/x.rs b/x.rs
--- a/x.rs
+++ b/x.rs
@@ -1,3 +1,2 @@
 fn main() {
-    let x = 1;
 }
";
        let idx = DiffPositionIndex::parse(diff);
        let del = idx
            .lines
            .iter()
            .position(|p| matches!(p.side_and_line, Some((DiffSide::Left, 2))))
            .expect("the -let x line");

        let c = idx.build_line_comment(del, del).expect("single-line built");
        assert_eq!(c.side, DiffSide::Left);
        assert_eq!(c.line, 2);
    }
}
