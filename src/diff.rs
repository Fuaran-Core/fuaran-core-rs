//! The structural tree diff: the skeleton-op script that turns one tree into
//! another over a shared id space, and the per-id classification read off the
//! same run of the emitter's passes — the inverse direction of
//! [`apply`](crate::ops::apply).
//!
//! The passes, in emission order (twin of Core's `Diff` emitter under no policy
//! and the plain scope, where every node may hold children):
//!
//! 0. every survivor whose SHELL (the node with its children emptied) differs
//!    under the caller's encoder: an `updateNode` carrying its `after` node;
//! 1. every added node, in `after`'s pre-order: an `insertChild` of its shell
//!    under its `after` parent;
//! 2. every `after` node whose child-id list changed: a `moveNode` for each
//!    survivor child whose parent changed;
//! 3. every removed node whose `before` parent survives: a `removeNode`;
//! 4. every parent of step 2 with more than one child whose order the steps
//!    above do not already leave right: a `reorderChildren` naming its `after`
//!    order.
//!
//! Applied to `before`, the script lands on `after`. A pair whose roots carry
//! different ids, or a tree holding an id twice, is refused.
//! Certified against Core's `diff/skeleton-diff.json`.

use crate::json::{escape_string, ordinal_cmp};
use crate::ops::Op;
use crate::prelude::*;
use crate::tree::Node;

/// Why no script exists between two trees (the two `Rejection` cases Core's
/// diff raises under no policy and the plain scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRefusal {
    /// The roots carry different ids (twin of `Rejection.RootIdMismatch`).
    RootIdMismatch { before: String, after: String },
    /// A tree holds this id twice: the first id seen a second time in pre-order,
    /// `before` judged first (twin of `Rejection.DuplicateId`).
    DuplicateId { id: String },
}

impl DiffRefusal {
    /// The refusal's canonical bytes, in the `apply/` family's rejection
    /// projection.
    pub fn encode(&self) -> String {
        match self {
            DiffRefusal::RootIdMismatch { before, after } => format!(
                "{{\"$type\":\"rootIdMismatch\",\"after\":{},\"before\":{}}}",
                escape_string(after),
                escape_string(before)
            ),
            DiffRefusal::DuplicateId { id } => {
                format!("{{\"$type\":\"duplicateId\",\"id\":{}}}", escape_string(id))
            }
        }
    }
}

/// How one id changed (twin of `Diff.ChangeKind`), declared in canonical order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// In `after`, not in `before`.
    Added,
    /// In `before`, not in `after` (a removed region's every id).
    Removed,
    /// A survivor under another parent.
    Moved { from: String, to: String },
    /// A survivor whose kind changed (reported instead of [`ChangeKind::Changed`]).
    KindChanged { from: String, to: String },
    /// A survivor of unchanged kind whose shell differs under the encoder.
    Changed,
    /// A survivor whose kept children stand in another relative order.
    Reordered,
}

impl ChangeKind {
    fn rank(&self) -> u8 {
        match self {
            ChangeKind::Added => 0,
            ChangeKind::Removed => 1,
            ChangeKind::Moved { .. } => 2,
            ChangeKind::KindChanged { .. } => 3,
            ChangeKind::Changed => 4,
            ChangeKind::Reordered => 5,
        }
    }

    /// The kind's canonical bytes (`$type`-discriminated).
    pub fn encode(&self) -> String {
        let pair = |tag: &str, from: &str, to: &str| {
            format!(
                "{{\"$type\":\"{tag}\",\"from\":{},\"to\":{}}}",
                escape_string(from),
                escape_string(to)
            )
        };
        match self {
            ChangeKind::Added => "{\"$type\":\"added\"}".to_string(),
            ChangeKind::Removed => "{\"$type\":\"removed\"}".to_string(),
            ChangeKind::Moved { from, to } => pair("moved", from, to),
            ChangeKind::KindChanged { from, to } => pair("kindChanged", from, to),
            ChangeKind::Changed => "{\"$type\":\"changed\"}".to_string(),
            ChangeKind::Reordered => "{\"$type\":\"reordered\"}".to_string(),
        }
    }
}

/// One classification entry (twin of `Diff.Change`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub kind: ChangeKind,
}

/// One run of the passes: the script and the changes, in the order recorded.
struct Passes {
    script: Vec<Op>,
    changes: Vec<Change>,
}

/// The tree's id → node and id → parent id maps, or the first repeated id.
struct Index<'a> {
    by_id: BTreeMap<&'a str, &'a Node>,
    parent_of: BTreeMap<&'a str, &'a str>,
    preorder: Vec<&'a Node>,
}

fn index_of(root: &Node) -> Result<Index<'_>, DiffRefusal> {
    let mut ix = Index {
        by_id: BTreeMap::new(),
        parent_of: BTreeMap::new(),
        preorder: Vec::new(),
    };
    let mut stack: Vec<(&Node, Option<&str>)> = vec![(root, None)];
    while let Some((n, parent)) = stack.pop() {
        if ix.by_id.insert(&n.id, n).is_some() {
            return Err(DiffRefusal::DuplicateId { id: n.id.clone() });
        }
        if let Some(p) = parent {
            ix.parent_of.insert(&n.id, p);
        }
        ix.preorder.push(n);
        for c in n.children.iter().rev() {
            stack.push((c, Some(&n.id)));
        }
    }
    Ok(ix)
}

fn shell(n: &Node) -> Node {
    Node::leaf(n.id.clone(), n.kind.clone())
}

fn child_ids(n: &Node) -> Vec<&str> {
    n.children.iter().map(|c| c.id.as_str()).collect()
}

fn passes(
    encode: Option<&dyn Fn(&Node) -> String>,
    before: &Node,
    after: &Node,
) -> Result<Passes, DiffRefusal> {
    if before.id != after.id {
        return Err(DiffRefusal::RootIdMismatch {
            before: before.id.clone(),
            after: after.id.clone(),
        });
    }
    let bix = index_of(before)?;
    let aix = index_of(after)?;
    let mut ops: Vec<Op> = Vec::new();
    let mut found: Vec<Change> = Vec::new();
    let mut note = |id: &str, kind: ChangeKind| {
        found.push(Change {
            id: id.to_string(),
            kind,
        })
    };

    // 0. Content: under the plain scope every node may hold children, so every
    //    content update goes first.
    if let Some(enc) = encode {
        for n in &aix.preorder {
            if let Some(b) = bix.by_id.get(n.id.as_str()) {
                let differs = enc(&shell(b)) != enc(&shell(n));
                if b.kind != n.kind {
                    note(
                        &n.id,
                        ChangeKind::KindChanged {
                            from: b.kind.clone(),
                            to: n.kind.clone(),
                        },
                    );
                } else if differs {
                    note(&n.id, ChangeKind::Changed);
                }
                if differs {
                    ops.push(Op::UpdateNode { node: (*n).clone() });
                }
            }
        }
    }

    // 1. Added nodes, as shells, top-down.
    for n in &aix.preorder {
        if !bix.by_id.contains_key(n.id.as_str()) {
            note(&n.id, ChangeKind::Added);
            if let Some(p) = aix.parent_of.get(n.id.as_str()) {
                ops.push(Op::InsertChild {
                    parent: p.to_string(),
                    node: shell(n),
                });
            }
        }
    }

    // 2. Reattach survivors whose parent changed; note reordered kept children.
    let mut reorder_parents: Vec<&str> = Vec::new();
    for p in &aix.preorder {
        let a_kids = child_ids(p);
        let b_node = bix.by_id.get(p.id.as_str());
        let b_kids: Option<Vec<&str>> = b_node.map(|b| child_ids(b));
        if b_kids.as_ref() == Some(&a_kids) {
            continue;
        }
        if let Some(bk) = &b_kids {
            let kept_before: Vec<&str> =
                bk.iter().copied().filter(|c| a_kids.contains(c)).collect();
            let kept_after: Vec<&str> = a_kids.iter().copied().filter(|c| bk.contains(c)).collect();
            if kept_before != kept_after {
                note(&p.id, ChangeKind::Reordered);
            }
        }
        for c in &p.children {
            if !bix.by_id.contains_key(c.id.as_str()) {
                continue;
            }
            match bix.parent_of.get(c.id.as_str()) {
                Some(bp) if *bp == p.id => {}
                bp => {
                    ops.push(Op::MoveNode {
                        target: c.id.clone(),
                        new_parent: p.id.clone(),
                    });
                    if let Some(bp) = bp {
                        note(
                            &c.id,
                            ChangeKind::Moved {
                                from: bp.to_string(),
                                to: p.id.clone(),
                            },
                        );
                    }
                }
            }
        }
        reorder_parents.push(&p.id);
    }

    // 3. Removals, at the top of each removed region.
    for n in &bix.preorder {
        if !aix.by_id.contains_key(n.id.as_str()) {
            note(&n.id, ChangeKind::Removed);
            if let Some(p) = bix.parent_of.get(n.id.as_str()) {
                if aix.by_id.contains_key(p) {
                    ops.push(Op::RemoveNode {
                        target: n.id.clone(),
                    });
                }
            }
        }
    }

    // 4. Order, where the order steps 1-3 leave is not the `after` order.
    for pid in reorder_parents {
        let Some(p) = aix.by_id.get(pid) else {
            continue;
        };
        if p.children.len() <= 1 {
            continue;
        }
        let a_kids = child_ids(p);
        let b_kids: Vec<&str> = bix.by_id.get(pid).map(|b| child_ids(b)).unwrap_or_default();
        let survives = |c: &&str| bix.by_id.contains_key(*c);
        let mut settled: Vec<&str> = b_kids
            .iter()
            .copied()
            .filter(|c| a_kids.contains(c))
            .collect();
        settled.extend(a_kids.iter().copied().filter(|c| !survives(c)));
        settled.extend(
            a_kids
                .iter()
                .copied()
                .filter(|c| survives(c) && !b_kids.contains(c)),
        );
        if settled != a_kids {
            ops.push(Op::ReorderChildren {
                parent: pid.to_string(),
                order: a_kids.iter().map(|c| c.to_string()).collect(),
            });
        }
    }

    Ok(Passes {
        script: ops,
        changes: found,
    })
}

/// The structural script from `before` to `after` (twin of `Diff.toOps` under no
/// policy and the plain scope): survivors keep `before`'s content.
pub fn to_ops(before: &Node, after: &Node) -> Result<Vec<Op>, DiffRefusal> {
    passes(None, before, after).map(|p| p.script)
}

/// The content-aware script (twin of `Diff.toOpsWith` under no policy and the
/// plain scope): [`to_ops`] plus an `updateNode` for every survivor whose shell
/// differs under `encode`, which should be injective over a node's own content.
pub fn to_ops_with(
    encode: &dyn Fn(&Node) -> String,
    before: &Node,
    after: &Node,
) -> Result<Vec<Op>, DiffRefusal> {
    passes(Some(encode), before, after).map(|p| p.script)
}

/// The per-id classification read off [`to_ops_with`]'s run (twin of
/// `Diff.changes` under no policy and the plain scope): ascending id (ordinal),
/// then [`ChangeKind`] declaration order. Refuses exactly where [`to_ops_with`]
/// does.
pub fn changes(
    encode: &dyn Fn(&Node) -> String,
    before: &Node,
    after: &Node,
) -> Result<Vec<Change>, DiffRefusal> {
    passes(Some(encode), before, after).map(|p| {
        let mut cs = p.changes;
        cs.sort_by(|a, b| ordinal_cmp(&a.id, &b.id).then(a.kind.rank().cmp(&b.kind.rank())));
        cs
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_lands_on_after() {
        let before = Node {
            id: "r".into(),
            kind: "doc".into(),
            children: vec![Node::leaf("a", "p"), Node::leaf("b", "p")],
        };
        let after = Node {
            id: "r".into(),
            kind: "doc".into(),
            children: vec![Node::leaf("c", "p"), Node::leaf("b", "q")],
        };
        let script = to_ops_with(&Node::encode, &before, &after).unwrap();
        let landed = script
            .iter()
            .try_fold(before, |t, op| crate::ops::apply(&t, op))
            .unwrap();
        assert_eq!(landed, after);
    }
}
