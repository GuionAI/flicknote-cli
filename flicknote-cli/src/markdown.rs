//! Lightweight markdown heading parser and section extractor.
//!
//! Provides heading extraction, tree building, section filtering, and
//! section extraction for structural markdown editing. Derived from
//! treemd's parser/document module (MIT license).

use sha2::{Digest, Sha256};

const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Derive `len` base62 chars from a SHA-256 hash of `input`, using 6 bits per char.
fn hash_to_base62(input: &str, len: usize) -> String {
    let hash = Sha256::digest(input.as_bytes());
    let n = u64::from_be_bytes(
        hash[..8]
            .try_into()
            .expect("SHA-256 digest is 32 bytes; first 8 bytes always fit u64"),
    );
    (0..len)
        .rev()
        .map(|i| BASE62[((n >> (i * 6)) % 62) as usize] as char)
        .collect()
}

/// Compute a stable 2-char base62 section ID from a heading line (e.g. "## Task 1").
pub(crate) fn section_id(heading_line: &str) -> String {
    hash_to_base62(heading_line.trim(), 2)
}

/// Compute section IDs for all headings, extending to 3 chars on collision.
///
/// On collision (same 2-char ID), a positional disambiguator is included in the
/// hash input so that two headings with identical text get distinct 3-char IDs.
pub(crate) fn assign_section_ids(heading_lines: &[String]) -> Vec<String> {
    let ids_2: Vec<String> = heading_lines.iter().map(|h| section_id(h)).collect();

    let mut counts = std::collections::HashMap::new();
    for id in &ids_2 {
        *counts.entry(id.clone()).or_insert(0u32) += 1;
    }

    heading_lines
        .iter()
        .enumerate()
        .zip(ids_2.iter())
        .map(|((idx, heading), id_2)| {
            if counts[id_2] > 1 {
                // Include position to disambiguate headings with identical text
                let disambiguated = format!("{}\x00{}", heading.trim(), idx);
                hash_to_base62(&disambiguated, 3)
            } else {
                id_2.clone()
            }
        })
        .collect()
}

/// A markdown document with its content and structure.
#[derive(Debug, Clone)]
pub(crate) struct Document {
    pub content: String,
    pub headings: Vec<Heading>,
}

/// A heading in a markdown document.
#[derive(Debug, Clone)]
pub(crate) struct Heading {
    /// Heading level (1 for #, 2 for ##, etc.)
    pub level: usize,
    /// Heading text content
    pub text: String,
    /// Byte offset where the heading starts in the source document
    pub offset: usize,
    /// Stable 2-char (or 3-char on collision) base62 section ID
    pub id: String,
}

/// A node in the heading tree (for box-drawing display).
#[derive(Debug, Clone)]
pub(crate) struct HeadingNode {
    pub heading: Heading,
    pub children: Vec<Self>,
}

/// Parse markdown content and extract headings with byte offsets.
///
/// Skips headings inside fenced code blocks (``` or ~~~).
pub(crate) fn parse_markdown(content: &str) -> Document {
    let mut headings = Vec::new();
    let mut in_code_block = false;

    for (offset, line) in line_offsets(content) {
        let trimmed = line.trim_start();

        // Track fenced code blocks
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        // Parse ATX headings: # Heading
        if let Some(rest) = trimmed.strip_prefix('#') {
            let mut level = 1usize;
            let mut chars = rest.chars();
            while chars.as_str().starts_with('#') {
                level += 1;
                chars.next();
            }
            // Must be followed by space or be end of line
            let remaining = chars.as_str();
            if level <= 6 && (remaining.is_empty() || remaining.starts_with(' ')) {
                let text = remaining.trim().to_string();
                if !text.is_empty() {
                    headings.push(Heading {
                        level,
                        text,
                        offset,
                        id: String::new(), // placeholder, filled below
                    });
                }
            }
        }
    }

    // Compute IDs with collision detection
    let heading_lines: Vec<String> = headings
        .iter()
        .map(|h| format!("{} {}", "#".repeat(h.level), h.text))
        .collect();
    let ids = assign_section_ids(&heading_lines);
    for (h, id) in headings.iter_mut().zip(ids) {
        h.id = id;
    }

    Document {
        content: content.to_string(),
        headings,
    }
}

/// Iterate over lines with their byte offsets.
fn line_offsets(content: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    content.split('\n').map(move |line| {
        let start = offset;
        offset += line.len() + 1; // +1 for the \n
        (start, line)
    })
}

impl Document {
    /// Build a hierarchical tree from flat heading list.
    pub(crate) fn build_tree(&self) -> Vec<HeadingNode> {
        let mut roots: Vec<HeadingNode> = Vec::new();
        // Stack of (level, index_path) to track nesting
        let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();

        for heading in &self.headings {
            let node = HeadingNode {
                heading: heading.clone(),
                children: Vec::new(),
            };

            // Pop until we find a parent with lower level
            while let Some(&(parent_level, _)) = stack.last() {
                if parent_level < heading.level {
                    break;
                }
                stack.pop();
            }

            if let Some((_, path)) = stack.last() {
                // Navigate to parent and add as child
                let path = path.clone();
                let parent = navigate_mut(&mut roots, &path);
                let idx = parent.children.len();
                parent.children.push(node);
                let mut child_path = path;
                child_path.push(idx);
                stack.push((heading.level, child_path));
            } else {
                // Root node
                let idx = roots.len();
                roots.push(node);
                stack.push((heading.level, vec![idx]));
            }
        }

        roots
    }
}

/// Navigate a tree to find a mutable reference to a node by index path.
///
/// Path format: [root_idx, child_idx, child_idx, ...]
/// First element indexes into the roots vec, subsequent elements index into children.
fn navigate_mut<'a>(roots: &'a mut [HeadingNode], path: &[usize]) -> &'a mut HeadingNode {
    let mut current = &mut roots[path[0]];
    for &idx in &path[1..] {
        current = &mut current.children[idx];
    }
    current
}

impl HeadingNode {
    /// Render as tree with box-drawing characters.
    pub(crate) fn render_box_tree(&self, prefix: &str, is_last: bool) -> String {
        let mut result = String::new();

        let connector = if is_last { "└─ " } else { "├─ " };
        let marker = "#".repeat(self.heading.level);
        let heading_display = if self.heading.level > 1 {
            format!("[{}] {} {}", self.heading.id, marker, self.heading.text)
        } else {
            format!("{} {}", marker, self.heading.text)
        };
        result.push_str(&format!("{}{}{}\n", prefix, connector, heading_display));

        let child_prefix = format!("{}{}   ", prefix, if is_last { " " } else { "│" });

        for (i, child) in self.children.iter().enumerate() {
            let is_last_child = i == self.children.len() - 1;
            result.push_str(&child.render_box_tree(&child_prefix, is_last_child));
        }

        result
    }
}

/// Counts leading `#` characters on a line if followed by a space (valid heading).
fn heading_level(line: &str) -> Option<usize> {
    if !line.starts_with('#') {
        return None;
    }
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if line.as_bytes().get(hashes) == Some(&b' ') {
        Some(hashes)
    } else {
        None
    }
}

/// Shift all headings in `content` so the shallowest heading lands at `target_level`,
/// preserving relative hierarchy. Non-heading lines are unchanged.
///
/// Examples (target_level = 3):
///   `## Intro / ### Sub`  →  `### Intro / #### Sub`  (offset +1)
///   `#### Deep / ##### Deeper`  →  `### Deep / #### Deeper`  (offset -1)
///   `### Right / #### Sub`  →  unchanged  (offset 0)
pub(crate) fn cap_heading_level(content: &str, target_level: usize) -> String {
    let min_level = content.lines().filter_map(heading_level).min();
    let Some(min_level) = min_level else {
        return content.to_string();
    };

    let offset = target_level as isize - min_level as isize;
    if offset == 0 {
        return content.to_string();
    }

    content
        .lines()
        .map(|line| match heading_level(line) {
            Some(hashes) => {
                let new_level = ((hashes as isize + offset) as usize).min(6);
                let text = &line[hashes..]; // includes the leading space
                format!("{}{}", "#".repeat(new_level), text)
            }
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace the body of a section (identified by its byte bounds) with new content.
///
/// Preserves the heading line. Heading-level cap is applied by the caller before this.
/// Returns `Err` if the heading line has no trailing newline (malformed document).
pub(crate) fn replace_section_body(
    content: &str,
    heading_start: usize,
    section_end: usize,
    new_body: &str,
) -> Result<String, String> {
    let heading_line_end = content[heading_start..]
        .find('\n')
        .map(|i| heading_start + i + 1)
        .ok_or_else(|| {
            "Section heading has no trailing newline — malformed document".to_string()
        })?;

    let before = &content[..heading_line_end]; // includes the heading line with \n
    let after = &content[section_end..];
    let result = if after.is_empty() {
        format!("{}\n{}", before.trim_end_matches('\n'), new_body)
    } else {
        format!(
            "{}\n\n{}\n\n{}",
            before.trim_end_matches('\n'),
            new_body,
            after.trim_start_matches('\n')
        )
    };
    Ok(result)
}

/// Render the full tree for a markdown content string (for post-mutation output).
pub(crate) fn render_tree(content: &str) -> String {
    let doc = parse_markdown(content);
    let tree = doc.build_tree();
    if tree.is_empty() {
        return "(no headings found)\n".to_string();
    }
    let mut out = String::new();
    for (i, node) in tree.iter().enumerate() {
        out.push_str(&node.render_box_tree("", i == tree.len() - 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_tree_includes_ids() {
        let content = "# Root\n\n## Alpha\n\nContent.\n\n## Beta\n\nContent.";
        let output = render_tree(content);
        assert!(
            output.contains('['),
            "render_tree should include ID brackets"
        );
        assert!(output.contains("] ## Alpha"));
        assert!(output.contains("] ## Beta"));
    }

    #[test]
    fn test_render_tree_no_headings() {
        let content = "Just some paragraph text with no headings.";
        let output = render_tree(content);
        assert_eq!(output, "(no headings found)\n");
    }

    #[test]
    fn test_parse_headings() {
        let md = "# Title\nSome content\n\n## Section 1\nMore content\n\n### Subsection\nDetails\n\n## Section 2\nEnd";
        let doc = parse_markdown(md);
        assert_eq!(doc.headings.len(), 4);
        assert_eq!(doc.headings[0].level, 1);
        assert_eq!(doc.headings[0].text, "Title");
        assert_eq!(doc.headings[1].level, 2);
        assert_eq!(doc.headings[1].text, "Section 1");
    }

    #[test]
    fn test_headings_in_code_blocks_ignored() {
        let md = "# Real\n\n```\n# Not a heading\n```\n\n## Also Real";
        let doc = parse_markdown(md);
        assert_eq!(doc.headings.len(), 2);
        assert_eq!(doc.headings[0].text, "Real");
        assert_eq!(doc.headings[1].text, "Also Real");
    }

    #[test]
    fn test_build_tree() {
        let md = "# Root\n\n## Child 1\n\n### Grandchild\n\n## Child 2";
        let doc = parse_markdown(md);
        let tree = doc.build_tree();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 2);
        assert_eq!(tree[0].children[0].children.len(), 1);
    }

    #[test]
    fn test_render_box_tree() {
        let md = "# Root\n\n## Child 1\n\n## Child 2";
        let doc = parse_markdown(md);
        let tree = doc.build_tree();
        let output = tree[0].render_box_tree("", true);
        assert!(output.contains("# Root"));
        assert!(output.contains("## Child 1"));
        assert!(output.contains("## Child 2"));
    }

    #[test]
    fn test_tree_shows_section_ids() {
        let content = "# Root\n\n## Alpha\n\nContent.\n\n## Beta\n\nContent.";
        let doc = parse_markdown(content);
        let tree = doc.build_tree();
        let output = tree[0].render_box_tree("", true);
        assert!(
            output.contains('['),
            "tree output should contain section ID brackets"
        );
        assert!(
            output.contains("] ## Alpha"),
            "Alpha heading should have ID prefix"
        );
        assert!(
            output.contains("] ## Beta"),
            "Beta heading should have ID prefix"
        );
        // Root heading (H1) should NOT have an ID prefix
        assert!(
            !output.contains("] # Root"),
            "root H1 should not have ID prefix"
        );
    }
}

#[cfg(test)]
mod section_id_tests {
    use super::*;

    #[test]
    fn test_section_id_stable() {
        let id1 = section_id("## Task 1: Add ObjectExists");
        let id2 = section_id("## Task 1: Add ObjectExists");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_section_id_length_is_two() {
        let id = section_id("## Some Heading");
        assert_eq!(id.len(), 2);
    }

    #[test]
    fn test_section_ids_differ_for_different_headings() {
        let id1 = section_id("## Task 1");
        let id2 = section_id("## Task 2");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_collision_extends_to_3_chars_and_is_distinct() {
        let headings = vec!["## Alpha".to_string(), "## Alpha".to_string()];
        let ids = assign_section_ids(&headings);
        assert!(
            ids.iter().all(|id| id.len() == 3),
            "all IDs should be 3 chars"
        );
        assert_ne!(ids[0], ids[1], "colliding headings must get distinct IDs");
    }

    #[test]
    fn test_no_collision_stays_2_chars() {
        let headings = vec![
            "## Alpha".to_string(),
            "## Beta".to_string(),
            "## Gamma".to_string(),
        ];
        let ids = assign_section_ids(&headings);
        assert!(
            ids.iter().all(|id| id.len() == 2),
            "non-colliding IDs must be exactly 2 chars"
        );
    }
}

#[cfg(test)]
mod cap_heading_tests {
    use super::*;

    #[test]
    fn test_cap_headings_h4_promoted_to_h3() {
        // H4 is the shallowest, so it becomes H3; H5 shifts to H4 (hierarchy preserved)
        let input = "Some text.\n\n#### Too Deep\n\nContent.\n\n##### Also Deep\n\nMore.";
        let result = cap_heading_level(input, 3);
        assert!(
            result.contains("### Too Deep"),
            "H4 should be promoted to H3"
        );
        assert!(
            result.contains("#### Also Deep"),
            "H5 should shift to H4 (one below H4→H3)"
        );
    }

    #[test]
    fn test_cap_headings_h3_unchanged() {
        let input = "Some text.\n\n### Already H3\n\nContent.";
        let result = cap_heading_level(input, 3);
        assert!(result.contains("### Already H3"), "H3 should be unchanged");
    }

    #[test]
    fn test_cap_headings_no_headings_unchanged() {
        let input = "Just paragraph text.\n\nMore text.";
        let result = cap_heading_level(input, 3);
        assert_eq!(
            result, input,
            "content without headings should be unchanged"
        );
    }

    #[test]
    fn test_cap_h1_demoted_to_h3() {
        let input = "# Top Level\n\nContent.";
        let result = cap_heading_level(input, 3);
        assert!(
            result.contains("### Top Level"),
            "H1 inside H2 section should demote to H3"
        );
        assert!(
            !result.lines().any(|l| l == "# Top Level"),
            "original H1 marker must not remain"
        );
        assert!(
            result.contains("Content."),
            "body text must survive unchanged"
        );
    }

    #[test]
    fn test_cap_h2_demoted_to_h3() {
        let input = "## Second Level\n\nContent.";
        let result = cap_heading_level(input, 3);
        assert!(
            result.contains("### Second Level"),
            "H2 inside H2 section should demote to H3"
        );
        assert!(
            result.contains("Content."),
            "body text must survive unchanged"
        );
    }

    #[test]
    fn test_cap_h3_unchanged_clamp() {
        let input = "### Third Level\n\nContent.";
        let result = cap_heading_level(input, 3);
        assert!(
            result.contains("### Third Level"),
            "H3 should remain unchanged"
        );
        assert!(
            result.contains("Content."),
            "body text must survive unchanged"
        );
    }

    #[test]
    fn test_cap_h4_promoted_to_h3_clamp() {
        let input = "#### Fourth Level\n\nContent.";
        let result = cap_heading_level(input, 3);
        assert!(
            result.contains("### Fourth Level"),
            "H4 should be promoted to H3"
        );
        assert!(
            result.contains("Content."),
            "body text must survive unchanged"
        );
    }

    #[test]
    fn test_cap_mixed_offset_shift_preserves_hierarchy() {
        // H1 is shallowest → becomes H3; H3 and H5 shift by same offset (+2)
        // Hierarchy is preserved: H1→H3, H3→H5, H5→H7
        let input = "# Intro\n\nPara.\n\n### Middle\n\nText.\n\n##### Deep\n\nMore.";
        let result = cap_heading_level(input, 3);
        assert!(result.contains("### Intro"), "H1 should shift to H3");
        assert!(result.contains("##### Middle"), "H3 should shift to H5");
        assert!(
            result.contains("###### Deep"),
            "H5 should shift to H7, clamped to H6"
        );
        assert!(
            !result.lines().any(|l| l == "# Intro"),
            "H1 marker must not remain"
        );
        assert!(result.contains("Para."), "body text must survive unchanged");
    }

    #[test]
    fn test_cap_target_level_1_shift_preserves_hierarchy() {
        // H2 is shallowest → becomes H1; H3 shifts to H2 (offset = -1)
        let input = "## Deep\n\nContent.\n\n### Deeper\n\nMore.";
        let result = cap_heading_level(input, 1);
        assert!(result.contains("# Deep"), "H2 should shift to H1");
        assert!(result.contains("## Deeper"), "H3 should shift to H2");
    }

    #[test]
    fn test_offset_shift_standalone_doc_into_h2_section() {
        // Real-world case: user writes standalone content starting at H1,
        // inserts into an H2 section (target=3). Full hierarchy shifts by +2.
        let input = "## Overview\n\nIntro.\n\n### Details\n\nText.\n\n#### Notes\n\nMore.";
        let result = cap_heading_level(input, 3);
        assert!(result.contains("### Overview"), "H2 → H3");
        assert!(result.contains("#### Details"), "H3 → H4");
        assert!(result.contains("##### Notes"), "H4 → H5");
        assert!(
            result.contains("Intro."),
            "body text must survive unchanged"
        );
    }

    #[test]
    fn test_cap_hash_without_space_passes_through() {
        // "#NoSpace" is not a valid heading — must not be altered
        let input = "#NoSpace\n\n## Real Heading\n\nContent.";
        let result = cap_heading_level(input, 3);
        assert!(
            result.contains("#NoSpace"),
            "#NoSpace should pass through unchanged"
        );
        assert!(
            result.contains("### Real Heading"),
            "valid H2 should demote to H3"
        );
    }
}

#[cfg(test)]
mod replace_section_body_tests {
    use super::*;

    fn alpha_bounds(content: &str) -> (usize, usize) {
        let doc = parse_markdown(content);
        let alpha_id = doc
            .headings
            .iter()
            .find(|h| h.text == "Alpha")
            .expect("Alpha heading not found")
            .id
            .clone();
        let idx = doc.headings.iter().position(|h| h.id == alpha_id).unwrap();
        let start = doc.headings[idx].offset;
        let end = doc
            .headings
            .iter()
            .skip(idx + 1)
            .find(|h| h.level <= doc.headings[idx].level)
            .map(|h| h.offset)
            .unwrap_or(content.len());
        (start, end)
    }

    #[test]
    fn test_replace_section_preserves_heading() {
        let original = "# Root\n\n## Alpha\n\nOld content.\n\n## Beta\n\nBeta content.\n";
        let (start, end) = alpha_bounds(original);
        let result = replace_section_body(original, start, end, "New content.").unwrap();

        assert!(result.contains("## Alpha"), "heading should be preserved");
        assert!(
            result.contains("New content."),
            "new content should be present"
        );
        assert!(
            !result.contains("Old content."),
            "old content should be gone"
        );
        assert!(
            result.contains("## Beta"),
            "other sections must be untouched"
        );
        assert!(
            result.contains("Beta content."),
            "other section content must be untouched"
        );
    }

    #[test]
    fn test_replace_section_last_section() {
        let original = "# Root\n\n## Alpha\n\nOld.\n";
        let (start, end) = alpha_bounds(original);
        let result = replace_section_body(original, start, end, "New.").unwrap();

        assert!(result.contains("## Alpha"), "heading preserved");
        assert!(result.contains("New."), "new content present");
        assert!(!result.contains("Old."), "old content gone");
    }

    #[test]
    fn test_replace_section_heading_no_newline_errors() {
        // Malformed: heading at very end with no \n
        let content = "## Alpha";
        let result = replace_section_body(content, 0, content.len(), "body");
        assert!(
            result.is_err(),
            "should error on heading with no trailing newline"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("malformed"),
            "error should mention malformed document"
        );
    }

    #[test]
    fn test_replace_h2_section_shifts_content_headings() {
        // Alpha is H2, target = H3. Replacement starts with H1 (shallowest) → offset +2.
        // H1 → H3, H2 → H4: relative hierarchy preserved.
        let original = "# Root\n\n## Alpha\n\nOld.\n\n## Beta\n\nBeta.";
        let (start, end) = alpha_bounds(original);
        let new_body_raw = "Intro.\n\n# Subsection\n\nOK.\n\n## Sub-sub\n\nNot OK.";
        let doc = parse_markdown(original);
        let section_level = doc
            .headings
            .iter()
            .find(|h| h.text == "Alpha")
            .unwrap()
            .level;
        let shifted = cap_heading_level(new_body_raw, section_level + 1);
        let result = replace_section_body(original, start, end, &shifted).unwrap();

        assert!(result.contains("### Subsection"), "H1 should shift to H3");
        assert!(result.contains("#### Sub-sub"), "H2 should shift to H4");
        assert!(
            !result.lines().any(|l| l == "# Subsection"),
            "original H1 must not remain"
        );
    }
}
