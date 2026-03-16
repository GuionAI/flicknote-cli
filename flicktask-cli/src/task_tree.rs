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

        // Sort children by UUID for deterministic output
        for list in children.values_mut() {
            list.sort();
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
