//! Markdown section parser for protocol files.
//!
//! Parses ATX-heading-structured markdown into a flat map of heading-path → content,
//! suitable for section-level drift comparison.

use std::collections::BTreeMap;

/// Parsed representation of a markdown file: a map from heading path to content text.
///
/// The heading path is a list of heading texts, e.g. `["Runtime Adapters", "Generic Fields"]`.
/// Content is the text between that heading and the next heading of equal or higher level,
/// normalized for comparison (trimmed, trailing whitespace collapsed).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedFile {
    /// Relative path of the file (for diagnostics).
    pub relative_path: String,
    /// Ordered section paths in document order.
    pub section_order: Vec<SectionPath>,
    /// Map from section path to normalized content.
    pub sections: BTreeMap<SectionPath, String>,
    /// Raw lines for line-level diagnostics.
    pub line_count: usize,
}

/// A section path: ordered list of heading texts.
pub type SectionPath = Vec<String>;

/// Parse a markdown string into sections.
///
/// # How it works
///
/// 1. Split into lines, tracking ATX headings (#, ##, ###, etc.).
/// 2. Build a section stack: when a heading at level N is encountered,
///    pop the stack back to depth N-1, then push the new heading.
/// 3. All non-heading lines between headings are accumulated as content
///    for the current deepest section.
/// 4. Content is normalized: trimmed per-line, trailing blank lines removed.
pub fn parse(relative_path: &str, source: &str) -> ParsedFile {
    let mut sections: BTreeMap<SectionPath, String> = BTreeMap::new();
    let mut section_order: Vec<SectionPath> = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new(); // (level, heading_text)
    let mut current_content: Vec<&str> = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    // Flush accumulated content for the current section path
    fn flush(
        stack: &[(usize, String)],
        content: &mut Vec<&str>,
        sections: &mut BTreeMap<SectionPath, String>,
        section_order: &mut Vec<SectionPath>,
    ) {
        let path: SectionPath = stack.iter().map(|(_, h)| h.clone()).collect();
        if path.is_empty() {
            content.clear();
            return;
        }
        let text = normalize_content(content);
        if !text.is_empty() || sections.contains_key(&path) {
            // Only record non-empty sections, or sections that already exist
        }
        if !sections.contains_key(&path) {
            section_order.push(path.clone());
        }
        sections.insert(path, text);
        content.clear();
    }

    for line in &lines {
        if let Some((level, heading)) = parse_heading(line) {
            // Flush current content before changing section
            flush(
                &stack,
                &mut current_content,
                &mut sections,
                &mut section_order,
            );

            // Pop stack to the right depth
            while stack.last().map_or(false, |(l, _)| *l >= level) {
                stack.pop();
            }
            stack.push((level, heading));
        } else {
            current_content.push(line);
        }
    }

    // Flush remaining content
    flush(
        &stack,
        &mut current_content,
        &mut sections,
        &mut section_order,
    );

    ParsedFile {
        relative_path: relative_path.to_string(),
        section_order,
        sections,
        line_count: lines.len(),
    }
}

/// Parse an ATX heading line. Returns `(level, heading_text)` if it's a heading.
fn parse_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }

    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }

    // Must have a space after the #s (CommonMark spec)
    let after_hashes = &trimmed[level..];
    if after_hashes.is_empty() || !after_hashes.starts_with(' ') {
        return None;
    }

    // Extract heading text: strip leading #s, trailing #s, and whitespace
    let heading = after_hashes.trim().trim_end_matches('#').trim().to_string();

    if heading.is_empty() {
        return None;
    }

    Some((level, heading))
}

/// Normalize content for comparison: trim each line, strip leading/trailing blank lines.
fn normalize_content(lines: &[&str]) -> String {
    let trimmed: Vec<&str> = lines.iter().map(|l| l.trim_end()).collect();

    // Find first non-empty line
    let start = trimmed
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(trimmed.len());

    // Find last non-empty line
    let end = trimmed
        .iter()
        .rposition(|l| !l.is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);

    if start >= end {
        return String::new();
    }

    trimmed[start..end].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_heading_with_content() {
        let input = "# Title\n\nSome content here.\n\nMore content.\n";
        let parsed = parse("test.md", input);

        assert_eq!(parsed.section_order, vec![vec!["Title".to_string()]]);
        assert_eq!(
            parsed
                .sections
                .get(&vec!["Title".to_string()])
                .map(|s| s.as_str()),
            Some("Some content here.\n\nMore content.")
        );
    }

    #[test]
    fn parse_multi_level_sections() {
        let input = "\
# Top
top content

## Middle
middle content

### Leaf
leaf content

## Middle 2
middle 2 content
";
        let parsed = parse("test.md", input);

        assert_eq!(parsed.sections.len(), 4);
        assert_eq!(
            parsed
                .sections
                .get(&vec!["Top".to_string()])
                .map(|s| s.as_str()),
            Some("top content")
        );
        assert_eq!(
            parsed
                .sections
                .get(&vec!["Top".to_string(), "Middle".to_string()])
                .map(|s| s.as_str()),
            Some("middle content")
        );
        assert_eq!(
            parsed
                .sections
                .get(&vec![
                    "Top".to_string(),
                    "Middle".to_string(),
                    "Leaf".to_string()
                ])
                .map(|s| s.as_str()),
            Some("leaf content")
        );
        assert_eq!(
            parsed
                .sections
                .get(&vec!["Top".to_string(), "Middle 2".to_string()])
                .map(|s| s.as_str()),
            Some("middle 2 content")
        );
    }

    #[test]
    fn parse_ignores_setext_headings() {
        // Setext headings (underlined with === or ---) are not parsed as sections.
        // They are treated as content of the parent section.
        let input = "# Title\n\nSome text\n=======\n\nMore text\n";
        let parsed = parse("test.md", input);

        assert_eq!(parsed.sections.len(), 1);
        let content = parsed.sections.get(&vec!["Title".to_string()]).unwrap();
        assert!(content.contains("======="));
    }

    #[test]
    fn parse_heading_with_trailing_hashes() {
        let input = "# Title ##\n\nContent\n";
        let parsed = parse("test.md", input);
        assert_eq!(parsed.section_order, vec![vec!["Title".to_string()]]);
    }

    #[test]
    fn parse_heading_without_space_is_not_heading() {
        let input = "#NotAHeading\n\nContent\n";
        let parsed = parse("test.md", input);
        // The #NotAHeading line becomes content under no section
        assert!(parsed.sections.is_empty());
    }

    #[test]
    fn parse_code_blocks_preserved_as_content() {
        let input = "\
# Section

```rust
fn main() {
    println!(\"hello\");
}
```

After code block.
";
        let parsed = parse("test.md", input);
        let content = parsed.sections.get(&vec!["Section".to_string()]).unwrap();
        assert!(content.contains("```rust"));
        assert!(content.contains("fn main()"));
        assert!(content.contains("After code block."));
    }

    #[test]
    fn parse_empty_file() {
        let parsed = parse("empty.md", "");
        assert!(parsed.sections.is_empty());
        assert_eq!(parsed.line_count, 0);
    }

    #[test]
    fn parse_section_reorder_same_level() {
        // Section at same level creates a new sibling, not a child
        let input = "\
# A
content a

# B
content b
";
        let parsed = parse("test.md", input);
        assert_eq!(parsed.sections.len(), 2);
        assert!(parsed.sections.contains_key(&vec!["A".to_string()]));
        assert!(parsed.sections.contains_key(&vec!["B".to_string()]));
    }

    #[test]
    fn parse_sibling_sections_at_level_2() {
        let input = "\
# Top

## First
first content

## Second
second content
";
        let parsed = parse("test.md", input);
        assert_eq!(parsed.sections.len(), 3); // Top, First, Second
        assert!(parsed
            .sections
            .contains_key(&vec!["Top".to_string(), "First".to_string()]));
        assert!(parsed
            .sections
            .contains_key(&vec!["Top".to_string(), "Second".to_string()]));
    }

    #[test]
    fn parse_normalizes_trailing_blank_lines() {
        let input = "# Title\n\ncontent\n\n\n";
        let parsed = parse("test.md", input);
        let content = parsed.sections.get(&vec!["Title".to_string()]).unwrap();
        assert_eq!(content, "content");
    }
}
