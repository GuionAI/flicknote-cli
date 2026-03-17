//! Fractional-index position helpers for sibling ordering.

use fractional_index::FractionalIndex;

/// Generate the position string for a new task appended at the end of its siblings.
/// `last_pos` is the position of the current last sibling (if any).
pub fn append_position(last_pos: Option<&str>) -> anyhow::Result<String> {
    match last_pos {
        Some(last) => {
            let last_idx = FractionalIndex::from_string(last)
                .map_err(|e| anyhow::anyhow!("Invalid stored position {last:?}: {e}"))?;
            Ok(FractionalIndex::new_after(&last_idx).to_string())
        }
        None => Ok(FractionalIndex::default().to_string()),
    }
}

/// Generate the position string for a new task prepended at the start.
/// `first_pos` is the position of the current first sibling (if any).
pub fn prepend_position(first_pos: Option<&str>) -> anyhow::Result<String> {
    match first_pos {
        Some(first) => {
            let first_idx = FractionalIndex::from_string(first)
                .map_err(|e| anyhow::anyhow!("Invalid stored position {first:?}: {e}"))?;
            Ok(FractionalIndex::new_before(&first_idx).to_string())
        }
        // No existing siblings — create a position before the default.
        // This avoids collision with append_position(None) which returns default().
        None => {
            let default_idx = FractionalIndex::default();
            Ok(FractionalIndex::new_before(&default_idx).to_string())
        }
    }
}

/// Generate the position string for a task inserted between two siblings.
/// Both `before_pos` and `after_pos` must be valid position strings.
pub fn between_position(before_pos: &str, after_pos: &str) -> anyhow::Result<String> {
    let before_idx = FractionalIndex::from_string(before_pos)
        .map_err(|e| anyhow::anyhow!("Invalid before position: {e}"))?;
    let after_idx = FractionalIndex::from_string(after_pos)
        .map_err(|e| anyhow::anyhow!("Invalid after position: {e}"))?;
    let between = FractionalIndex::new_between(&before_idx, &after_idx).ok_or_else(|| {
        anyhow::anyhow!("Cannot create position between {before_pos:?} and {after_pos:?}")
    })?;
    Ok(between.to_string())
}

/// Generate N sequential position strings (for `flicktask plan`).
pub fn sequential_positions(n: usize) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }
    let mut positions = Vec::with_capacity(n);
    let first = FractionalIndex::default();
    positions.push(first.to_string());
    let mut prev = first;
    for _ in 1..n {
        let next = FractionalIndex::new_after(&prev);
        positions.push(next.to_string());
        prev = next;
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_append_first() {
        let pos = append_position(None).unwrap();
        assert!(!pos.is_empty());
    }

    #[test]
    fn test_append_after_existing() {
        let first = append_position(None).unwrap();
        let second = append_position(Some(&first)).unwrap();
        assert!(
            second > first,
            "second ({second}) should be after first ({first})"
        );
    }

    #[test]
    fn test_prepend_before_existing() {
        let first = append_position(None).unwrap();
        let before = prepend_position(Some(&first)).unwrap();
        assert!(
            before < first,
            "before ({before}) should be before first ({first})"
        );
    }

    #[test]
    fn test_prepend_none_differs_from_append_none() {
        let appended = append_position(None).unwrap();
        let prepended = prepend_position(None).unwrap();
        assert_ne!(
            appended, prepended,
            "prepend(None) must differ from append(None)"
        );
        assert!(
            prepended < appended,
            "prepend(None) ({prepended}) should be before append(None) ({appended})"
        );
    }

    #[test]
    fn test_between() {
        let a = append_position(None).unwrap();
        let b = append_position(Some(&a)).unwrap();
        let mid = between_position(&a, &b).unwrap();
        assert!(mid > a, "mid ({mid}) should be after a ({a})");
        assert!(mid < b, "mid ({mid}) should be before b ({b})");
    }

    #[test]
    fn test_between_invalid_input() {
        assert!(between_position("not-hex", "80").is_err());
        assert!(between_position("80", "not-hex").is_err());
    }

    #[test]
    fn test_append_invalid_stored_position() {
        assert!(
            append_position(Some("corrupt!!")).is_err(),
            "corrupt stored position must propagate as error"
        );
    }

    #[test]
    fn test_prepend_invalid_stored_position() {
        assert!(
            prepend_position(Some("corrupt!!")).is_err(),
            "corrupt stored position must propagate as error"
        );
    }

    #[test]
    fn test_sequential_positions_ordering() {
        let positions = sequential_positions(5);
        assert_eq!(positions.len(), 5);
        for i in 1..positions.len() {
            assert!(
                positions[i] > positions[i - 1],
                "position[{i}] ({}) should be after position[{}] ({})",
                positions[i],
                i - 1,
                positions[i - 1]
            );
        }
    }

    #[test]
    fn test_sequential_positions_zero() {
        assert!(sequential_positions(0).is_empty());
    }
}
