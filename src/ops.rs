//! The skeleton ops and their apply — Core's tree-op contract over the
//! [`Node`] witness, with no domain policy: every node may hold
//! children, and a rejection names its class and the id it is about.
//!
//! | Op | Refused when, in this order |
//! |---|---|
//! | `insertChild` | an id of the graft is already in the tree or repeats inside the graft (`duplicateId`, the first in pre-order); the parent is absent (`unknownNode`) |
//! | `removeNode` | the target is the root (`cannotRemoveRoot`); it is absent (`unknownNode`) |
//! | `moveNode` | the target is the root (`cannotRemoveRoot`); the target, then the new parent, is absent (`unknownNode`); the new parent is the target or below it (`wouldNestUnderSelf`) |
//! | `reorderChildren` | the parent is absent (`unknownNode`); the order is not a permutation of its children's ids (`reorderMismatch`) |
//! | `updateNode` | no node has the payload's id (`unknownNode`); the kind is rewritten in place and the children kept |
//! | `batch` | any member is refused: all or nothing, and the member's own rejection is the answer |
//!
//! An inserted node is appended to its parent's children, and a moved one to its
//! new parent's. Ops and rejections are `$type`-discriminated canonical JSON.
//! Certified against Core's `apply/skeleton-apply.json`.

use crate::decode::{self, Cursor, DecodeCode, DecodeError};
use crate::json::{JsonLimits, escape_string, render_array};
use crate::prelude::*;
use crate::tree::Node;

/// A skeleton op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    InsertChild { parent: String, node: Node },
    RemoveNode { target: String },
    MoveNode { target: String, new_parent: String },
    ReorderChildren { parent: String, order: Vec<String> },
    Batch(Vec<Op>),
    UpdateNode { node: Node },
}

/// Why an op was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    DuplicateId { id: String },
    UnknownNode { target: String },
    CannotRemoveRoot,
    WouldNestUnderSelf { target: String },
    ReorderMismatch { parent: String },
}

const OP_TAGS: &[&str] = &[
    "insertChild",
    "removeNode",
    "moveNode",
    "reorderChildren",
    "batch",
    "updateNode",
];

impl Op {
    /// The op's canonical bytes.
    pub fn encode(&self) -> String {
        match self {
            Op::InsertChild { parent, node } => format!(
                "{{\"$type\":\"insertChild\",\"node\":{},\"parent\":{}}}",
                node.encode(),
                escape_string(parent)
            ),
            Op::RemoveNode { target } => format!(
                "{{\"$type\":\"removeNode\",\"target\":{}}}",
                escape_string(target)
            ),
            Op::MoveNode { target, new_parent } => format!(
                "{{\"$type\":\"moveNode\",\"newParent\":{},\"target\":{}}}",
                escape_string(new_parent),
                escape_string(target)
            ),
            Op::ReorderChildren { parent, order } => format!(
                "{{\"$type\":\"reorderChildren\",\"order\":{},\"parent\":{}}}",
                render_array(&order.iter().map(|s| escape_string(s)).collect::<Vec<_>>()),
                escape_string(parent)
            ),
            Op::Batch(ops) => format!(
                "{{\"$type\":\"batch\",\"ops\":{}}}",
                render_array(&ops.iter().map(Op::encode).collect::<Vec<_>>())
            ),
            Op::UpdateNode { node } => {
                format!("{{\"$type\":\"updateNode\",\"node\":{}}}", node.encode())
            }
        }
    }

    /// Read an op, each refusal at its path.
    pub fn decode(at: &Cursor<'_>) -> Result<Op, DecodeError> {
        let s = |name: &str| -> Result<String, DecodeError> {
            Ok(at.field(name)?.string()?.to_string())
        };
        Ok(match at.tag("$type", OP_TAGS)? {
            "insertChild" => Op::InsertChild {
                parent: s("parent")?,
                node: Node::decode(&at.field("node")?)?,
            },
            "removeNode" => Op::RemoveNode {
                target: s("target")?,
            },
            "moveNode" => Op::MoveNode {
                target: s("target")?,
                new_parent: s("newParent")?,
            },
            "reorderChildren" => Op::ReorderChildren {
                parent: s("parent")?,
                order: at
                    .field("order")?
                    .items()?
                    .iter()
                    .map(|c| c.string().map(str::to_string))
                    .collect::<Result<_, _>>()?,
            },
            "batch" => Op::Batch(
                at.field("ops")?
                    .items()?
                    .iter()
                    .map(Op::decode)
                    .collect::<Result<_, _>>()?,
            ),
            "updateNode" => Op::UpdateNode {
                node: Node::decode(&at.field("node")?)?,
            },
            other => {
                return Err(at.fail(DecodeCode::UnknownTag, format!("unknown op '{other}'")));
            }
        })
    }

    /// Read an op from its JSON text.
    pub fn parse(text: &str) -> Result<Op, DecodeError> {
        let v = decode::parse(text, &JsonLimits::STRICT)?;
        Op::decode(&Cursor::root(&v))
    }
}

impl Rejection {
    /// The rejection's canonical bytes — its class and the id it is about.
    pub fn encode(&self) -> String {
        match self {
            Rejection::DuplicateId { id } => {
                format!("{{\"$type\":\"duplicateId\",\"id\":{}}}", escape_string(id))
            }
            Rejection::UnknownNode { target } => format!(
                "{{\"$type\":\"unknownNode\",\"target\":{}}}",
                escape_string(target)
            ),
            Rejection::CannotRemoveRoot => "{\"$type\":\"cannotRemoveRoot\"}".to_string(),
            Rejection::WouldNestUnderSelf { target } => format!(
                "{{\"$type\":\"wouldNestUnderSelf\",\"target\":{}}}",
                escape_string(target)
            ),
            Rejection::ReorderMismatch { parent } => format!(
                "{{\"$type\":\"reorderMismatch\",\"parent\":{}}}",
                escape_string(parent)
            ),
        }
    }
}

fn unknown(id: &str) -> Rejection {
    Rejection::UnknownNode {
        target: id.to_string(),
    }
}

/// The first id of `graft`, in pre-order, already in `tree` or repeated inside
/// the graft itself.
fn first_duplicate(tree: &Node, graft: &Node) -> Option<String> {
    let mut seen: BTreeSet<&str> = tree.ids().into_iter().collect();
    graft
        .ids()
        .into_iter()
        .find(|id| !seen.insert(id))
        .map(str::to_string)
}

/// Detach the node with `id` from below `n`; `None` when it is not below `n`.
fn detach(n: &mut Node, id: &str) -> Option<Node> {
    if let Some(i) = n.children.iter().position(|c| c.id == id) {
        return Some(n.children.remove(i));
    }
    n.children.iter_mut().find_map(|c| detach(c, id))
}

/// Apply `op` to `tree`: the new tree, or the rejection. `tree` is never changed.
pub fn apply(tree: &Node, op: &Op) -> Result<Node, Rejection> {
    let mut out = tree.clone();
    apply_in_place(&mut out, op)?;
    Ok(out)
}

/// Apply `op` to `tree` in place. On a rejection the tree may be partly changed
/// (a batch member applied before the refused one); [`apply`] is the
/// all-or-nothing form.
fn apply_in_place(tree: &mut Node, op: &Op) -> Result<(), Rejection> {
    match op {
        Op::InsertChild { parent, node } => {
            if let Some(d) = first_duplicate(tree, node) {
                return Err(Rejection::DuplicateId { id: d });
            }
            let p = tree.find_mut(parent).ok_or_else(|| unknown(parent))?;
            p.children.push(node.clone());
            Ok(())
        }
        Op::RemoveNode { target } => {
            if *target == tree.id {
                return Err(Rejection::CannotRemoveRoot);
            }
            detach(tree, target)
                .map(|_| ())
                .ok_or_else(|| unknown(target))
        }
        Op::MoveNode { target, new_parent } => {
            if *target == tree.id {
                return Err(Rejection::CannotRemoveRoot);
            }
            let moved = tree.find(target).ok_or_else(|| unknown(target))?;
            tree.find(new_parent).ok_or_else(|| unknown(new_parent))?;
            if moved.find(new_parent).is_some() {
                return Err(Rejection::WouldNestUnderSelf {
                    target: target.clone(),
                });
            }
            let node = detach(tree, target).expect("the target was found below the root");
            tree.find_mut(new_parent)
                .expect("the new parent is outside the moved subtree")
                .children
                .push(node);
            Ok(())
        }
        Op::ReorderChildren { parent, order } => {
            let p = tree.find_mut(parent).ok_or_else(|| unknown(parent))?;
            let mut current: Vec<&str> = p.children.iter().map(|c| c.id.as_str()).collect();
            let mut wanted: Vec<&str> = order.iter().map(String::as_str).collect();
            current.sort_unstable();
            wanted.sort_unstable();
            if current != wanted {
                return Err(Rejection::ReorderMismatch {
                    parent: parent.clone(),
                });
            }
            let mut old = core::mem::take(&mut p.children);
            for id in order {
                let i = old
                    .iter()
                    .position(|c| c.id == *id)
                    .expect("the order is a permutation of the children");
                p.children.push(old.remove(i));
            }
            Ok(())
        }
        Op::Batch(ops) => {
            for o in ops {
                apply_in_place(tree, o)?;
            }
            Ok(())
        }
        Op::UpdateNode { node } => {
            let target = tree.find_mut(&node.id).ok_or_else(|| unknown(&node.id))?;
            target.kind = node.kind.clone();
            Ok(())
        }
    }
}

// ─── The footprint ───────────────────────────────────────────────────────────

/// The read/write footprint of an op-script (twin of Core's `Footprint`), over
/// node ids. Core's record also carries slot reads and writes; no skeleton op
/// accesses a slot, so they are always empty here and are not carried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Footprint {
    /// Ids whose existence an op depends on.
    pub reads: BTreeSet<String>,
    /// Parents whose child list an op changes.
    pub structure_writes: BTreeSet<String>,
    /// Ids authored, destroyed, relocated or rewritten.
    pub content_writes: BTreeSet<String>,
    /// Ids removed, moved or rewritten under a parent the script cannot name.
    pub unknown_parent_writes: BTreeSet<String>,
}

impl Footprint {
    /// The union of two footprints (twin of `Footprint.union`).
    pub fn union(mut self, other: Footprint) -> Footprint {
        self.reads.extend(other.reads);
        self.structure_writes.extend(other.structure_writes);
        self.content_writes.extend(other.content_writes);
        self.unknown_parent_writes
            .extend(other.unknown_parent_writes);
        self
    }

    /// Does the footprint change any child list, named or not?
    pub(crate) fn writes_structure(&self) -> bool {
        !self.structure_writes.is_empty() || !self.unknown_parent_writes.is_empty()
    }
}

fn set_of<'a>(ids: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    ids.into_iter().map(str::to_string).collect()
}

fn footprint_of_op(op: &Op) -> Footprint {
    match op {
        Op::InsertChild { parent, node } => {
            let inserted = set_of(node.ids());
            let mut reads = inserted.clone();
            reads.insert(parent.clone());
            Footprint {
                reads,
                structure_writes: set_of([parent.as_str()]),
                content_writes: inserted,
                ..Footprint::default()
            }
        }
        Op::RemoveNode { target } => Footprint {
            reads: set_of([target.as_str()]),
            content_writes: set_of([target.as_str()]),
            unknown_parent_writes: set_of([target.as_str()]),
            ..Footprint::default()
        },
        Op::MoveNode { target, new_parent } => Footprint {
            reads: set_of([target.as_str(), new_parent.as_str()]),
            structure_writes: set_of([new_parent.as_str()]),
            content_writes: set_of([target.as_str()]),
            unknown_parent_writes: set_of([target.as_str()]),
        },
        Op::ReorderChildren { parent, order } => {
            let mut reads = set_of(order.iter().map(String::as_str));
            reads.insert(parent.clone());
            Footprint {
                reads,
                structure_writes: set_of([parent.as_str()]),
                ..Footprint::default()
            }
        }
        Op::Batch(inner) => footprint(inner),
        Op::UpdateNode { node } => Footprint {
            reads: set_of([node.id.as_str()]),
            content_writes: set_of([node.id.as_str()]),
            unknown_parent_writes: set_of([node.id.as_str()]),
            ..Footprint::default()
        },
    }
}

/// The footprint of an op-script: the union of each op's (twin of
/// `Ops.footprint Policy.none` over the plain scope). Conservative by contract: a
/// remove, a move and an update carry an unknown-parent write, so each is
/// dependent on every concurrent structural write.
pub fn footprint(ops: &[Op]) -> Footprint {
    ops.iter().fold(Footprint::default(), |acc, op| {
        acc.union(footprint_of_op(op))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Node {
        Node {
            id: "root".into(),
            kind: "doc".into(),
            children: vec![
                Node {
                    id: "a".into(),
                    kind: "section".into(),
                    children: vec![Node::leaf("a1", "para")],
                },
                Node::leaf("b", "section"),
            ],
        }
    }

    #[test]
    fn a_batch_is_all_or_nothing() {
        let op = Op::Batch(vec![
            Op::RemoveNode {
                target: "a1".into(),
            },
            Op::RemoveNode {
                target: "nope".into(),
            },
        ]);
        assert_eq!(apply(&tree(), &op), Err(unknown("nope")));
    }

    #[test]
    fn an_op_round_trips_through_its_canonical_bytes() {
        let op = Op::Batch(vec![
            Op::MoveNode {
                target: "a1".into(),
                new_parent: "b".into(),
            },
            Op::ReorderChildren {
                parent: "root".into(),
                order: vec!["b".into(), "a".into()],
            },
        ]);
        assert_eq!(Op::parse(&op.encode()), Ok(op));
    }
}
