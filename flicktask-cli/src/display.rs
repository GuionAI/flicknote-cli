use std::collections::HashMap;

use taskchampion::{Task, Uuid};

use crate::task_tree::TaskTree;

/// Walk a task subtree with box-drawing connectors.
/// `format_node` receives `(uuid, task)` and returns the line text (after the connector).
/// Returns `None` to skip a node entirely.
pub fn print_subtree<F>(
    all_tasks: &HashMap<Uuid, Task>,
    tree: &TaskTree,
    uuid: Uuid,
    max_depth: Option<usize>,
    depth: usize,
    indent: &str,
    format_node: &F,
) where
    F: Fn(Uuid, &Task) -> Option<String>,
{
    if max_depth.is_some_and(|max| depth >= max) {
        return;
    }

    let children = tree.children(uuid);
    let count = children.len();
    for (i, child_uuid) in children.iter().enumerate() {
        let Some(task) = all_tasks.get(child_uuid) else {
            continue;
        };
        let Some(line) = format_node(*child_uuid, task) else {
            continue;
        };

        let is_last = i == count - 1;
        let connector = if is_last { "└─" } else { "├─" };
        let next_indent = if is_last {
            format!("{indent}   ")
        } else {
            format!("{indent}│  ")
        };

        println!("{indent}{connector} {line}");
        print_subtree(
            all_tasks,
            tree,
            *child_uuid,
            max_depth,
            depth + 1,
            &next_indent,
            format_node,
        );
    }
}
