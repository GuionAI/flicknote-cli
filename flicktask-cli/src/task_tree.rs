use std::collections::{HashMap, HashSet};

use taskchampion::{Status, Task, Uuid};

use crate::ids::short_id;

pub struct TaskTree {
    children: HashMap<Uuid, Vec<Uuid>>,
    parent: HashMap<Uuid, Uuid>,
    all_uuids: Vec<Uuid>,
}

impl TaskTree {
    pub fn from_tasks(tasks: &HashMap<Uuid, Task>) -> Self {
        let mut children: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        let mut parent_map: HashMap<Uuid, Uuid> = HashMap::new();

        // Initialize children list for all tasks
        for uuid in tasks.keys() {
            children.entry(*uuid).or_default();
        }

        for (uuid, task) in tasks {
            if let Some(parent_str) = task.get_value("parent") {
                match Uuid::parse_str(parent_str) {
                    Ok(parent_uuid) => {
                        parent_map.insert(*uuid, parent_uuid);
                        children.entry(parent_uuid).or_default().push(*uuid);
                    }
                    Err(_) => {
                        eprintln!(
                            "Warning: task {} has invalid parent UUID {:?} — treating as root",
                            short_id(uuid),
                            parent_str
                        );
                    }
                }
            }
        }

        // Sort children by (position, entry) — positioned first in order,
        // then unpositioned by creation time, then by UUID as final tiebreaker.
        for list in children.values_mut() {
            list.sort_by(|a, b| {
                let task_a = tasks.get(a);
                let task_b = tasks.get(b);
                let pos_a = task_a.and_then(|t| t.get_value("position"));
                let pos_b = task_b.and_then(|t| t.get_value("position"));

                match (pos_a, pos_b) {
                    // Both have positions — lexicographic compare
                    (Some(pa), Some(pb)) => pa.cmp(pb),
                    // Only a has position — a comes first
                    (Some(_), None) => std::cmp::Ordering::Less,
                    // Only b has position — b comes first
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    // Neither has position — sort by entry timestamp, then UUID
                    (None, None) => {
                        let entry_a = task_a.and_then(|t| t.get_value("entry"));
                        let entry_b = task_b.and_then(|t| t.get_value("entry"));
                        entry_a.cmp(&entry_b).then_with(|| a.cmp(b))
                    }
                }
            });
        }

        Self {
            children,
            parent: parent_map,
            all_uuids: tasks.keys().copied().collect(),
        }
    }

    pub fn children(&self, uuid: Uuid) -> Vec<Uuid> {
        self.children.get(&uuid).cloned().unwrap_or_default()
    }

    /// Depth-first descendants (not including the root itself).
    /// Includes a visited-set guard to break on cyclic parent data.
    pub fn descendants(&self, uuid: Uuid) -> Vec<Uuid> {
        let mut result = Vec::new();
        let mut stack = vec![uuid];
        let mut visited = HashSet::new();
        visited.insert(uuid);

        while let Some(current) = stack.pop() {
            for child in self.children(current) {
                if visited.insert(child) {
                    result.push(child);
                    stack.push(child);
                } else {
                    eprintln!(
                        "Warning: cycle detected in task tree at {} — breaking traversal",
                        short_id(&child)
                    );
                }
            }
        }
        result
    }

    /// Tasks with no parent (root tasks).
    pub fn roots(&self) -> Vec<Uuid> {
        self.all_uuids
            .iter()
            .filter(|uuid| !self.parent.contains_key(*uuid))
            .copied()
            .collect()
    }

    /// Check if `ancestor` is an ancestor of `uuid` (walk up parent chain).
    /// Includes a visited-set guard to break on cyclic parent data.
    pub fn is_ancestor(&self, uuid: Uuid, ancestor: Uuid) -> bool {
        let mut current = uuid;
        let mut visited = HashSet::new();
        visited.insert(current);

        loop {
            match self.parent.get(&current).copied() {
                None => return false,
                Some(p) if p == ancestor => return true,
                Some(p) => {
                    if !visited.insert(p) {
                        eprintln!(
                            "Warning: cycle detected in task parent chain at {} — breaking",
                            short_id(&p)
                        );
                        return false;
                    }
                    current = p;
                }
            }
        }
    }

    /// Get the position values of siblings under a parent (in order).
    /// `parent` is `Some(uuid)` for a child task, `None` for root tasks.
    /// `exclude` optionally removes a UUID from results (for move operations —
    /// the task being moved should not appear in its own sibling list).
    /// Returns vec of `(uuid, position_string)` for siblings that have positions.
    pub fn sibling_positions(
        &self,
        parent: Option<Uuid>,
        tasks: &HashMap<Uuid, Task>,
        exclude: Option<Uuid>,
    ) -> Vec<(Uuid, String)> {
        let siblings = match parent {
            Some(p) => self.children(p),
            None => self.roots(),
        };
        siblings
            .into_iter()
            .filter(|uuid| exclude != Some(*uuid))
            .filter_map(|uuid| {
                let task = tasks.get(&uuid)?;
                let pos = task.get_value("position")?;
                Some((uuid, pos.to_string()))
            })
            .collect()
    }

    /// Returns 8-char hex IDs of pending direct children — shared guard for done/delete.
    pub fn pending_child_ids(&self, uuid: Uuid, all_tasks: &HashMap<Uuid, Task>) -> Vec<String> {
        self.children(uuid)
            .into_iter()
            .filter(|child_uuid| {
                all_tasks
                    .get(child_uuid)
                    .map(|t| matches!(t.get_status(), Status::Pending))
                    .unwrap_or(false)
            })
            .map(|u| short_id(&u))
            .collect()
    }
}
