/// Extract title from first markdown H1 heading (# Title).
/// Returns None if no heading found.
pub(crate) fn extract_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_title_normal_heading() {
        assert_eq!(
            extract_title("# My Title\nsome body"),
            Some("My Title".into())
        );
    }

    #[test]
    fn extract_title_no_heading() {
        assert_eq!(extract_title("just some text\nno heading here"), None);
    }

    #[test]
    fn extract_title_empty_h1() {
        assert_eq!(extract_title("# \nsome body"), None);
    }

    #[test]
    fn extract_title_h2_skipped() {
        assert_eq!(extract_title("## Not a title\n### Also not"), None);
    }

    #[test]
    fn extract_title_trims_whitespace() {
        assert_eq!(
            extract_title("#   Spaced Title   \nbody"),
            Some("Spaced Title".into())
        );
    }

    #[test]
    fn extract_title_first_h1_anywhere() {
        assert_eq!(
            extract_title("some preamble\n\n# First\n\n# Second"),
            Some("First".into()),
        );
    }
}
