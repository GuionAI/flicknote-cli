//! Lightweight markdown heading parser and section extractor.
//!
//! Provides heading extraction, tree building, section filtering, and
//! section extraction for structural markdown editing. Derived from
//! treemd's parser/document module (MIT license).

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
                    });
                }
            }
        }
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

    /// Get all headings matching a filter (case-insensitive contains).
    pub(crate) fn filter_headings(&self, filter: &str) -> Vec<&Heading> {
        let search = filter.to_lowercase();
        self.headings
            .iter()
            .filter(|h| h.text.to_lowercase().contains(&search))
            .collect()
    }

    /// Extract the content of a section by heading text (case-insensitive exact match).
    ///
    /// Returns the content between the heading line and the next same-or-higher-level heading.
    pub(crate) fn extract_section(&self, heading_text: &str) -> Option<String> {
        let heading_idx = self
            .headings
            .iter()
            .position(|h| h.text.to_lowercase() == heading_text.to_lowercase())?;

        let heading = &self.headings[heading_idx];
        let start = heading.offset;

        // Skip the heading line itself
        let content_start = self.content[start..]
            .find('\n')
            .map(|i| start + i + 1)
            .unwrap_or(start);

        // Find end: next heading at same or higher level
        let end = self
            .headings
            .iter()
            .skip(heading_idx + 1)
            .find(|h| h.level <= heading.level)
            .map(|h| h.offset)
            .unwrap_or(self.content.len());

        Some(self.content[content_start..end].trim().to_string())
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
        result.push_str(&format!(
            "{}{}{} {}\n",
            prefix, connector, marker, self.heading.text
        ));

        let child_prefix = format!("{}{}   ", prefix, if is_last { " " } else { "│" });

        for (i, child) in self.children.iter().enumerate() {
            let is_last_child = i == self.children.len() - 1;
            result.push_str(&child.render_box_tree(&child_prefix, is_last_child));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_extract_section() {
        let md = "# Main\n\n## Section A\nContent A\n\n## Section B\nContent B";
        let doc = parse_markdown(md);
        let content = doc.extract_section("Section A").unwrap();
        assert!(content.contains("Content A"));
        assert!(!content.contains("Content B"));
    }

    #[test]
    fn test_extract_section_at_end() {
        let md = "# First\n\n## Last Section\nFinal content\nMore lines";
        let doc = parse_markdown(md);
        let content = doc.extract_section("Last Section").unwrap();
        assert!(content.contains("Final content"));
        assert!(content.contains("More lines"));
    }

    #[test]
    fn test_filter_headings() {
        let md = "# Plan\n\n## Task 1: Do thing\n\n## Task 2: Do other\n\n## Summary";
        let doc = parse_markdown(md);
        let matches = doc.filter_headings("Task");
        assert_eq!(matches.len(), 2);
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
}
