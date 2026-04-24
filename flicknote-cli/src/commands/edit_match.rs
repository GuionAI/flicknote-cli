//! BEFORE/AFTER edit-mode parser and exact-match engine for `flicknote modify`.
//!
//! Ported from organon's `src edit` with markdown-appropriate simplifications:
//! - **Exact-only matching.** No fuzzy fallbacks (no trim-trailing, trim-both,
//!   or unicode-fold). Notes are prose — unicode dashes, quotes, and whitespace
//!   variations are legitimate content, not noise. Silent normalization would
//!   corrupt legitimate text.
//! - No reindent logic (not a code file domain).
//! - No CRLF/binary/size guards (notes are text-only, LF-only).
//!
//! ## Multi-match handling
//! organon's `src edit` errors on multiple matches with line numbers + snippets.
//! flicknote mirrors this: multi-match is an error by default (no `--all` flag).
//!
//! ## Atomicity
//! Atomicity is free: content is spliced in memory then written once via write_content.
//! Partial updates are never persisted. No transaction needed.
//!
//! ## Reference
//! organon `src edit` source: `/Users/neil/Code/guion-opensource/organon/internal/srcop/edit.go`

use flicknote_core::error::CliError;

const BEFORE_DELIM: &str = "===BEFORE===";
const AFTER_DELIM: &str = "===AFTER===";

/// Detect edit-mode stdin: first non-whitespace-only line is exactly `===BEFORE===`.
pub(crate) fn is_edit_mode(stdin: &str) -> bool {
    stdin
        .lines()
        .map(str::trim_end)
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| l.trim() == BEFORE_DELIM)
}

/// Parse a BEFORE/AFTER block. Returns `(before, after)` strings.
///
/// ## Errors
/// - no `===BEFORE===` found
/// - multiple `===BEFORE===` markers (lists 1-based line numbers)
/// - no `===AFTER===` found
/// - multiple `===AFTER===` markers (trailing-marker special-case for better error)
/// - empty BEFORE after trimming blank border lines
/// - identical BEFORE and AFTER (no-op)
///
/// Mirrors organon `parseEditInput` error wording.
pub(crate) fn parse_edit_input(input: &str) -> Result<(String, String), CliError> {
    // Normalize CRLF → LF for parsing consistency.
    let text = input.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();

    // Collect ALL line indices for each marker (1-based for error messages).
    let before_lines: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| trimmed_eq(l, BEFORE_DELIM))
        .map(|(i, _)| i + 1)
        .collect();

    let after_lines: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| trimmed_eq(l, AFTER_DELIM))
        .map(|(i, _)| i + 1)
        .collect();

    // Validate marker counts.
    match before_lines.len() {
        0 => {
            return Err(CliError::Other(format!("no {} found", BEFORE_DELIM)));
        }
        n if n > 1 => {
            return Err(CliError::Other(format!(
                "found {} lines matching {} (at lines {:?}). \
                 section headers, not tag pairs — use multiple `flicknote modify` calls for multiple edits.",
                n, BEFORE_DELIM, before_lines
            )));
        }
        _ => {}
    }

    match after_lines.len() {
        0 => {
            return Err(CliError::Other(format!("no {} found", AFTER_DELIM)));
        }
        2 if is_trailing_after_delimiter(&lines, &after_lines) => {
            return Err(CliError::Other(format!(
                "found 2 lines matching {} (at lines {:?}). \
                 This looks like a trailing {} — use section headers, not tag pairs.",
                AFTER_DELIM, after_lines, AFTER_DELIM
            )));
        }
        n if n > 1 => {
            return Err(CliError::Other(format!(
                "found {} lines matching {} (at lines {:?}). \
                 section headers, not tag pairs — use multiple `flicknote modify` calls for multiple edits.",
                n, AFTER_DELIM, after_lines
            )));
        }
        _ => {}
    }

    let before_idx = before_lines[0] - 1; // 0-based
    let after_idx = after_lines[0] - 1; // 0-based

    // Slice between markers (exclusive of marker lines).
    let before_block: Vec<&str> = lines[before_idx + 1..after_idx].to_vec();
    let after_block: Vec<&str> = lines[after_idx + 1..].to_vec();

    // Trim leading/trailing blank lines from each block.
    let before_block = trim_blank_border_lines(&before_block);
    let after_block = trim_blank_border_lines(&after_block);

    if before_block.is_empty() {
        return Err(CliError::Other("BEFORE section is empty".to_string()));
    }

    // Join with newlines — BEFORE always gets trailing newline per organon convention.
    let before = before_block.join("\n") + "\n";
    let after = if after_block.is_empty() {
        String::new()
    } else {
        after_block.join("\n") + "\n"
    };

    if before == after {
        return Err(CliError::Other(
            "old and new text are identical (no-op)".to_string(),
        ));
    }

    Ok((before, after))
}

/// Result of a successful exact match.
#[derive(Debug, Clone)]
pub(crate) struct MatchInfo {
    /// Byte offset where the BEFORE match starts in source.
    pub start: usize,
    /// Byte offset where the BEFORE match ends (exclusive) in source.
    pub end: usize,
}

/// Find exactly one occurrence of `before` in `source`. Returns byte range.
///
/// ## Errors
/// - Zero matches: error body ends with `closest_region()` output (agent self-correction hint).
/// - Multiple matches: error lists 1-based line numbers + 60-char snippets of each match,
///   and advises adding surrounding context to disambiguate.
///
/// Uses EXACT byte matching only — no fuzzy passes. Mirrors organon's multi-match error format.
pub(crate) fn find_unique(source: &str, before: &str) -> Result<MatchInfo, CliError> {
    // Exact match only.
    let first = source.find(before);

    match first {
        Some(start) => {
            let end = start + before.len();

            // Count lines before `start` for absolute line numbers.
            let first_line_num = source[..start].matches('\n').count() + 1;

            // Check for duplicates — search for additional occurrences.
            let mut pos = end;
            let mut extra_count = 0;
            let mut extra_sites: Vec<(usize, usize, String)> = Vec::new();

            while let Some(next) = source[pos..].find(before) {
                let abs = pos + next;
                // Count newlines within the substring + offset for absolute line number.
                let line_num = source[pos..abs].matches('\n').count()
                    + source[..pos].matches('\n').count()
                    + 1;
                let snippet = snippet_from_line(source, line_num);
                extra_sites.push((line_num, abs, snippet));
                extra_count += 1;
                pos = abs + before.len();
            }

            if extra_count > 0 {
                // Multiple matches → error with all line numbers.
                let all_sites: Vec<(usize, String)> =
                    std::iter::once((first_line_num, snippet_from_line(source, first_line_num)))
                        .chain(extra_sites.into_iter().map(|(ln, _, sn)| (ln, sn)))
                        .collect();

                let total = extra_count + 1;
                let mut msg = format!("found {} matches:\n", total);
                for (line_num, snippet) in &all_sites {
                    msg.push_str(&format!("  line {}: {}\n", line_num, snippet));
                }
                msg.push_str("\nadd surrounding context to disambiguate");
                return Err(CliError::Other(msg));
            }

            Ok(MatchInfo { start, end })
        }
        None => Err(CliError::Other(format!(
            "text not found\nClosest region:\n{}",
            closest_region(source, before)
        ))),
    }
}

/// Apply a verified edit to source — splice `after` into the `[start, end)` byte range.
pub(crate) fn splice(source: &str, m: &MatchInfo, after: &str) -> String {
    let mut out = String::with_capacity(source.len() - (m.end - m.start) + after.len());
    out.push_str(&source[..m.start]);
    out.push_str(after);
    out.push_str(&source[m.end..]);
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

/// True when `line` trimmed of trailing whitespace equals `expected` trimmed.
fn trimmed_eq(line: &str, expected: &str) -> bool {
    line.trim_end() == expected
}

/// True when the second `===AFTER===` marker is followed only by blank lines.
/// Used for the "trailing marker" special-case error message.
fn is_trailing_after_delimiter(lines: &[&str], after_lines: &[usize]) -> bool {
    if after_lines.len() != 2 {
        return false;
    }
    let second_idx = after_lines[1] - 1; // 1-based → 0-based
    for line in lines.iter().skip(second_idx + 1) {
        if !line.trim().is_empty() {
            return false;
        }
    }
    true
}

/// Remove leading and trailing blank lines from a slice of lines.
fn trim_blank_border_lines<'a>(lines: &'a [&'a str]) -> Vec<&'a str> {
    let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(0);
    let end = lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0)
        .max(start);
    lines[start..end].to_vec()
}

/// Extract a 60-char snippet from line `line_num` (1-based) of source.
fn snippet_from_line(source: &str, line_num: usize) -> String {
    let line_idx = line_num.saturating_sub(1);
    source
        .lines()
        .nth(line_idx)
        .map(|l| {
            if l.len() > 60 {
                format!("{}…", &l[..60])
            } else {
                l.to_string()
            }
        })
        .unwrap_or_default()
}

/// Build a "closest region" hint for not-found errors.
///
/// Slides a window of BEFORE's line count through source and returns the
/// highest-scoring window (most lines in common with BEFORE) with 1-based
/// line numbers. Helps agents self-correct stale BEFOREs.
///
/// Mirrors organon `closestRegion` output format:
/// ```text
/// text not found in <scope>
///
/// Closest region:
///   N: <line content>
///   N+1: <line content>
/// ```
fn closest_region(source: &str, before: &str) -> String {
    if source.is_empty() {
        return "(source file is empty)".to_string();
    }

    let source_lines: Vec<&str> = source.split('\n').collect();
    let trimmed = before.trim_end_matches('\n');
    let n_old = if trimmed.is_empty() {
        0
    } else {
        trimmed.split('\n').count()
    };

    if n_old == 0 {
        return "(empty search text)".to_string();
    }

    if source_lines.len() < n_old {
        return format!(
            "(source has {} lines, search text has {})",
            source_lines.len(),
            n_old
        );
    }

    // Build normalized set of BEFORE lines for comparison.
    let old_lines: Vec<&str> = if trimmed.is_empty() {
        vec![]
    } else {
        trimmed.split('\n').collect()
    };
    let old_set: std::collections::HashSet<String> =
        old_lines.iter().map(|l| l.trim().to_string()).collect();

    let mut best_score = 0isize;
    let mut best_start = -1isize;

    for i in 0..=source_lines.len() - n_old {
        let score: isize = (0..n_old)
            .filter(|&j| old_set.contains(source_lines[i + j].trim()))
            .count() as isize;
        if score > best_score {
            best_score = score;
            best_start = i as isize;
        }
    }

    if best_score <= 0 {
        return "(no similar region found — BEFORE shares no lines with note)".to_string();
    }

    if best_start < 0 {
        return "(no similar region found — BEFORE shares no lines with note)".to_string();
    }

    let start = best_start as usize;
    let end = (start + n_old).min(source_lines.len());

    let mut sb = String::new();
    for (i, line) in source_lines.iter().enumerate().take(end).skip(start) {
        sb.push_str(&format!("{:>4}: {}\n", i + 1, line));
    }
    sb
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_edit_input ────────────────────────────────────────────────────────

    #[test]
    fn test_parse_valid_single_block() {
        let input = "===BEFORE===\nold\ntext\n===AFTER===\nnew\ntext";
        let (before, after) = parse_edit_input(input).unwrap();
        assert_eq!(before, "old\ntext\n");
        assert_eq!(after, "new\ntext\n");
    }

    #[test]
    fn test_parse_multiline_before_after() {
        let input = "===BEFORE===\nline1\nline2\nline3\n===AFTER===\nlineA\nlineB";
        let (before, after) = parse_edit_input(input).unwrap();
        assert_eq!(before, "line1\nline2\nline3\n");
        assert_eq!(after, "lineA\nlineB\n");
    }

    #[test]
    fn test_parse_no_before_marker() {
        let result = parse_edit_input("old text\n===AFTER===\nnew text");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("===BEFORE==="));
    }

    #[test]
    fn test_parse_no_after_marker() {
        let result = parse_edit_input("===BEFORE===\nold text");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("===AFTER==="));
    }

    #[test]
    fn test_parse_two_before_markers() {
        let input = "===BEFORE===\ntext\n===BEFORE===\nother\n===AFTER===\nnew";
        let result = parse_edit_input(input);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("found 2 lines matching"));
        assert!(msg.contains("===BEFORE==="));
    }

    #[test]
    fn test_parse_trailing_after_marker() {
        // Second ===AFTER=== followed only by blank lines → trailing marker error.
        let input = "===BEFORE===\nold\n===AFTER===\nnew\n\n===AFTER===\n\n\n";
        let result = parse_edit_input(input);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("trailing"));
    }

    #[test]
    fn test_parse_empty_before_block() {
        // Adjacent markers with no content between → empty BEFORE.
        let input = "===BEFORE===\n===AFTER===\nnew";
        let result = parse_edit_input(input);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("BEFORE section is empty"));
    }

    #[test]
    fn test_parse_identical_before_after() {
        let input = "===BEFORE===\nsame\ntext\n===AFTER===\nsame\ntext";
        let result = parse_edit_input(input);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("no-op"));
    }

    #[test]
    fn test_parse_trims_blank_border_lines() {
        // Leading blank lines stripped from BEFORE, trailing from both.
        let input = "===BEFORE===\n\n\n  \nold\n   \n===AFTER===\n   \n\nnew";
        let (before, after) = parse_edit_input(input).unwrap();
        assert_eq!(before, "old\n");
        assert_eq!(after, "new\n");
    }

    // ── is_edit_mode ─────────────────────────────────────────────────────────

    #[test]
    fn test_is_edit_mode_true() {
        assert!(is_edit_mode("===BEFORE===\nold\n===AFTER===\nnew"));
    }

    #[test]
    fn test_is_edit_mode_leading_blank_lines() {
        assert!(is_edit_mode("\n\n===BEFORE===\nold\n===AFTER===\nnew"));
    }

    #[test]
    fn test_is_edit_mode_with_whitespace() {
        // We trim before comparing, so leading whitespace on the marker line is OK.
        assert!(is_edit_mode("  ===BEFORE===\nold\n===AFTER===\nnew"));
    }

    #[test]
    fn test_is_edit_mode_false_plain_content() {
        assert!(!is_edit_mode("just some text"));
    }

    #[test]
    fn test_is_edit_mode_false_empty() {
        assert!(!is_edit_mode(""));
        assert!(!is_edit_mode("   \n  \n"));
    }

    // ── find_unique ───────────────────────────────────────────────────────────

    #[test]
    fn test_find_unique_unique_match() {
        let source = "hello world\nfoo bar baz\ngoodbye";
        let m = find_unique(source, "foo bar baz").unwrap();
        assert_eq!(m.start, 12);
        assert_eq!(m.end, 23);
    }

    #[test]
    fn test_find_unique_start_of_file() {
        let source = "hello world\ngoodbye";
        let m = find_unique(source, "hello world").unwrap();
        assert_eq!(m.start, 0);
        assert_eq!(m.end, 11);
    }

    #[test]
    fn test_find_unique_end_of_file() {
        let source = "hello\nworld";
        let m = find_unique(source, "world").unwrap();
        assert_eq!(m.start, 6);
        assert_eq!(m.end, 11);
    }

    #[test]
    fn test_find_unique_not_found() {
        let source = "hello world";
        let result = find_unique(source, "nonexistent");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("not found"), "msg={}", msg);
        assert!(msg.contains("Closest region:"), "msg={}", msg);
    }

    #[test]
    fn test_find_unique_not_found_empty_source() {
        let result = find_unique("", "anything");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("(source file is empty)"));
    }

    #[test]
    fn test_find_unique_multiple_matches() {
        let source = "foo bar\nfoo bar\nfoo bar";
        let result = find_unique(source, "foo bar");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("found 3 matches:"));
        assert!(msg.contains("  line 1: foo bar"));
        assert!(msg.contains("  line 2: foo bar"));
        assert!(msg.contains("  line 3: foo bar"));
        assert!(msg.contains("add surrounding context to disambiguate"));
    }

    #[test]
    fn test_find_unique_multiple_matches_snippet_truncation() {
        // Long lines are truncated to 60 chars.
        let long_line = "a".repeat(80);
        let source = format!("{}\n{}", long_line, long_line);
        let result = find_unique(&source, &long_line);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("aaaaa…")); // 60 chars + ellipsis
        assert!(!msg.contains(&"a".repeat(61))); // no 61-char untruncated
    }

    #[test]
    fn test_find_unique_section_scope_offset() {
        // Verify that MatchInfo.start is relative to the slice, not absolute.
        // The absolute offset adjustment happens in the caller (modify.rs).
        let source = "# Root\n\n## Alpha\n\nfoo bar baz\n\n## Beta\n\nqux";
        // Search within the Alpha section slice [start..end].
        let doc = crate::markdown::parse_markdown(source);
        let alpha_bounds =
            super::super::util::find_section(&doc, &doc.headings[1].id, "test").unwrap();
        let slice = &source[alpha_bounds.start..alpha_bounds.end];
        let m = find_unique(slice, "foo bar baz").unwrap();
        // m.start is relative to slice (Alpha section body), not absolute.
        assert!(m.start >= alpha_bounds.start);
        assert!(m.end <= alpha_bounds.end);
        // Verify the match is actually in the slice.
        assert_eq!(&slice[m.start..m.end], "foo bar baz");
    }

    // ── splice ────────────────────────────────────────────────────────────────

    #[test]
    fn test_splice_basic() {
        let source = "hello world";
        let m = MatchInfo { start: 6, end: 11 };
        let result = splice(source, &m, "rust");
        assert_eq!(result, "hello rust");
    }

    #[test]
    fn test_splice_at_start() {
        let source = "hello world";
        let m = MatchInfo { start: 0, end: 5 };
        let result = splice(source, &m, "hi");
        assert_eq!(result, "hi world");
    }

    #[test]
    fn test_splice_at_end() {
        let source = "hello world";
        let m = MatchInfo { start: 6, end: 11 };
        let result = splice(source, &m, "earth");
        assert_eq!(result, "hello earth");
    }

    #[test]
    fn test_splice_multiline() {
        let source = "line1\nline2\nline3\nline4";
        let m = MatchInfo {
            start: 6,
            end: 12, // "line2\n"
        };
        let result = splice(source, &m, "replaced\n");
        assert_eq!(result, "line1\nreplaced\nline3\nline4");
    }

    // ── closest_region ────────────────────────────────────────────────────────

    #[test]
    fn test_closest_region_empty_source() {
        let result = closest_region("", "some text");
        assert_eq!(result, "(source file is empty)");
    }

    #[test]
    fn test_closest_region_empty_before() {
        let result = closest_region("some content", "");
        assert_eq!(result, "(empty search text)");
    }

    #[test]
    fn test_closest_region_source_smaller_than_before() {
        let result = closest_region("a\nb", "a\nb\nc\nd\ne");
        assert!(result.contains("source has 2 lines"));
        assert!(result.contains("search text has 5"));
    }

    #[test]
    fn test_closest_region_format() {
        // Verify right-aligned 4-char line number + colon + space.
        // Test with content that actually appears in source (line 2 and 3 share keywords).
        let source = "alpha\nbeta\ngamma\ndelta";
        let result = closest_region(source, "beta\ngamma");
        assert!(result.contains("   2:"), "result={}", result);
        assert!(result.contains("   3:"), "result={}", result);
    }

    #[test]
    fn test_closest_region_finds_best_window() {
        // Source has two similar windows; closest_region should pick the one
        // with more lines in common with BEFORE.
        let source = "alpha beta\ngamma delta\nepsilon zeta\ntheta iota";
        // BEFORE matches "alpha beta" (1 line) and "epsilon zeta" (1 line) equally,
        // but the 2-line window "gamma delta\nepsilon zeta" shares more context.
        let before = "gamma delta\nepsilon zeta";
        let result = closest_region(source, before);
        // The window starting at line 2 should win.
        assert!(result.contains("  2: gamma delta"));
        assert!(result.contains("  3: epsilon zeta"));
    }
}
