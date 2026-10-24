//! The op-DAG: content-addressed nodes, each an op by an actor over a set of
//! parents. A node's id is
//!
//! ```text
//! sha256Hex( <parent ids sorted ordinally, joined by ","> ++ "|" ++ <actor> ++ "|" ++ <op> )
//! ```
//!
//! over the actor's canonical text and the op's ([`DagOp::canonical`]), so the id
//! is a function of the parent SET and the same write deduplicates to one node.
//! The DAG holds no state: a state is what replaying an order of its ops from an
//! initial state gives, with the domain's apply passed in.
//!
//! Orders are Kahn drains, the ready node with the smallest id first, so every
//! order is total and a function of the node set; a parent the DAG does not hold
//! constrains nothing. Certified against Core's `dag/skeleton-dag.json`.

use crate::chain::{Actor, encode_actor};
use crate::json::ordinal_cmp;
use crate::prelude::*;
use crate::sha256::sha256_hex;

/// An op a DAG can address: its canonical text is what the id hashes.
pub trait DagOp {
    fn canonical(&self) -> String;
}

impl DagOp for String {
    fn canonical(&self) -> String {
        self.clone()
    }
}

impl DagOp for crate::ops::Op {
    fn canonical(&self) -> String {
        self.encode()
    }
}

/// One node: its id, its parents in the order first written, its actor and op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagNode<Op> {
    pub id: String,
    pub parents: Vec<String>,
    pub actor: Actor,
    pub op: Op,
}

/// The content id of a node of `parents`, `actor` and an op of canonical text `op`.
pub fn node_id(parents: &[String], actor: &Actor, op: &str) -> String {
    let mut sorted: Vec<&str> = parents.iter().map(String::as_str).collect();
    sorted.sort_by(|a, b| ordinal_cmp(a, b));
    let mut preimage = sorted.join(",");
    preimage.push('|');
    preimage.push_str(&encode_actor(actor));
    preimage.push('|');
    preimage.push_str(op);
    sha256_hex(&preimage)
}

/// Why a write was refused; nothing is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendFault {
    /// A parent is `""` — the genesis marker, not a node id (genesis is no parents).
    EmptyParentId,
    /// A parent carries the `,` the id's pre-image joins parents with.
    CommaInParentId(String),
    /// A parent the DAG does not hold.
    UnknownParent(String),
    /// The id is held by a node of different content.
    ContentIdCollision(String),
}

impl AppendFault {
    /// The fault's canonical bytes: `$type`, and the offending id as `parentId`
    /// or `nodeId` (the reference exporter's `faultJ`).
    pub fn encode(&self) -> String {
        use crate::json::escape_string;
        match self {
            AppendFault::EmptyParentId => "{\"$type\":\"emptyParentId\"}".to_string(),
            AppendFault::CommaInParentId(p) => format!(
                "{{\"$type\":\"commaInParentId\",\"parentId\":{}}}",
                escape_string(p)
            ),
            AppendFault::UnknownParent(p) => format!(
                "{{\"$type\":\"unknownParent\",\"parentId\":{}}}",
                escape_string(p)
            ),
            AppendFault::ContentIdCollision(id) => format!(
                "{{\"$type\":\"contentIdCollision\",\"nodeId\":{}}}",
                escape_string(id)
            ),
        }
    }
}

/// The nodes a drain could not place, because they lie on or below a cycle, in id
/// order. Only a hand-built or unverified load can hold one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cyclic {
    pub unplaced: Vec<String>,
}

/// Why a replay of one head refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayFault<R> {
    UnknownHead(String),
    Cyclic(Cyclic),
    /// The domain refused the op of this node, the first in replay order.
    Rejected(String, R),
}

/// Why a replay of the whole DAG refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayAllFault<R> {
    Cyclic(Cyclic),
    Rejected(String, R),
}

/// Why an integrity walk stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagBreakReason {
    ContentIdMismatch,
    MissingParent,
}

impl DagBreakReason {
    /// The reason's canonical spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            DagBreakReason::ContentIdMismatch => "content-id mismatch (tampered node)",
            DagBreakReason::MissingParent => "missing parent",
        }
    }
}

/// The first integrity fault: the node, the reason, and the expected and found
/// values (the recomputed id and the stored one; `""` and the absent parent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagBreak {
    pub node: String,
    pub reason: DagBreakReason,
    pub expected: String,
    pub got: String,
}

/// A DAG of ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dag<Op> {
    nodes: BTreeMap<String, DagNode<Op>>,
}

impl<Op> Default for Dag<Op> {
    fn default() -> Self {
        Dag {
            nodes: BTreeMap::new(),
        }
    }
}

impl<Op: DagOp> Dag<Op> {
    pub fn new() -> Self {
        Self::default()
    }

    /// A DAG of stored nodes, their ids trusted as stored — what a load does
    /// before it verifies. [`Dag::first_break`] says whether they hold.
    pub fn from_nodes(nodes: Vec<DagNode<Op>>) -> Self {
        Dag {
            nodes: nodes.into_iter().map(|n| (n.id.clone(), n)).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&DagNode<Op>> {
        self.nodes.get(id)
    }

    /// Append `op` as a child of every id in `parents` (`[]` is a genesis node).
    /// The parents are judged in the order given, the first refusal returned:
    /// `""`, then a comma-bearing id, then an id the DAG does not hold. They are
    /// stored as given; the id sorts them, so their order changes no id. A write
    /// identical to a held node records nothing and answers that node's id.
    pub fn append_on(
        &mut self,
        actor: Actor,
        op: Op,
        parents: Vec<String>,
    ) -> Result<String, AppendFault> {
        for p in &parents {
            if p.is_empty() {
                return Err(AppendFault::EmptyParentId);
            }
            if p.contains(',') {
                return Err(AppendFault::CommaInParentId(p.clone()));
            }
            if !self.nodes.contains_key(p) {
                return Err(AppendFault::UnknownParent(p.clone()));
            }
        }
        let text = op.canonical();
        let id = node_id(&parents, &actor, &text);
        if let Some(existing) = self.nodes.get(&id) {
            let same = sorted(&existing.parents) == sorted(&parents)
                && existing.actor == actor
                && existing.op.canonical() == text;
            return if same {
                Ok(id)
            } else {
                Err(AppendFault::ContentIdCollision(id))
            };
        }
        self.nodes.insert(
            id.clone(),
            DagNode {
                id: id.clone(),
                parents,
                actor,
                op,
            },
        );
        Ok(id)
    }

    /// [`Dag::append_on`] over `heads` deduplicated and sorted ordinally.
    pub fn merge_all(
        &mut self,
        actor: Actor,
        op: Op,
        heads: Vec<String>,
    ) -> Result<String, AppendFault> {
        let mut parents: Vec<String> = dedup(&heads).into_iter().map(String::from).collect();
        parents.sort_by(|a, b| ordinal_cmp(a, b));
        self.append_on(actor, op, parents)
    }

    /// The nodes that are no node's parent, in id order.
    pub fn heads(&self) -> Vec<String> {
        let parents: BTreeSet<&str> = self
            .nodes
            .values()
            .flat_map(|n| n.parents.iter().map(String::as_str))
            .collect();
        let mut out: Vec<String> = self
            .nodes
            .keys()
            .filter(|id| !parents.contains(id.as_str()))
            .cloned()
            .collect();
        out.sort_by(|a, b| ordinal_cmp(a, b));
        out
    }

    /// The Kahn drain over the nodes `keep` admits, the ready node with the
    /// smallest id (UTF-16 ordinal) first; a parent `keep` refuses constrains
    /// nothing. `(placed, unplaced)`, the second in id order. Twin of `drainNodes`.
    pub(crate) fn drain(&self, keep: impl Fn(&str) -> bool) -> (Vec<String>, Vec<String>) {
        let mut indeg: BTreeMap<&str, usize> = BTreeMap::new();
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (id, n) in &self.nodes {
            if !keep(id) {
                continue;
            }
            let held: Vec<&str> = n
                .parents
                .iter()
                .map(String::as_str)
                .filter(|p| self.nodes.contains_key(*p) && keep(p))
                .collect();
            indeg.insert(id, held.len());
            for p in held {
                children.entry(p).or_default().push(id);
            }
        }
        // Ordered by UTF-16 code units, the ordinal order every host sorts ids by.
        let key = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
        let mut ready: BTreeMap<Vec<u16>, &str> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| (key(id), *id))
            .collect();
        let mut placed: Vec<String> = Vec::new();
        while let Some((_, id)) = ready.pop_first() {
            placed.push(id.to_string());
            if let Some(kids) = children.get(id) {
                for k in kids {
                    let d = indeg.get_mut(k).expect("every child has an in-degree");
                    *d -= 1;
                    if *d == 0 {
                        ready.insert(key(k), k);
                    }
                }
            }
        }
        let done: BTreeSet<&str> = placed.iter().map(String::as_str).collect();
        let mut unplaced: Vec<String> = indeg
            .keys()
            .filter(|id| !done.contains(*id))
            .map(|id| id.to_string())
            .collect();
        unplaced.sort_by(|a, b| ordinal_cmp(a, b));
        (placed, unplaced)
    }

    fn ordered(drained: (Vec<String>, Vec<String>)) -> Result<Vec<String>, Cyclic> {
        match drained {
            (placed, unplaced) if unplaced.is_empty() => Ok(placed),
            (_, unplaced) => Err(Cyclic { unplaced }),
        }
    }

    /// The whole DAG in one total order: parents first, the smallest ready id next.
    pub fn total_order(&self) -> Result<Vec<String>, Cyclic> {
        Self::ordered(self.drain(|_| true))
    }

    /// Every node's Lamport depth: 0 with no held parent, else one past its
    /// deepest held parent.
    pub fn ranks(&self) -> Result<BTreeMap<String, usize>, Cyclic> {
        let order = self.total_order()?;
        let mut ranks: BTreeMap<String, usize> = BTreeMap::new();
        for id in order {
            let r = self.nodes[&id]
                .parents
                .iter()
                .filter_map(|p| ranks.get(p).map(|rp| rp + 1))
                .max()
                .unwrap_or(0);
            ranks.insert(id, r);
        }
        Ok(ranks)
    }

    /// The ancestor closure of `roots`: each root plus every transitive parent the
    /// DAG holds; an id it does not hold contributes nothing. Twin of `closureOf`.
    pub(crate) fn closure_of<'a>(
        &'a self,
        roots: impl IntoIterator<Item = &'a str>,
    ) -> BTreeSet<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<&str> = roots.into_iter().collect();
        while let Some(id) = stack.pop() {
            if seen.contains(id) {
                continue;
            }
            if let Some(n) = self.nodes.get(id) {
                seen.insert(id.to_string());
                stack.extend(n.parents.iter().map(String::as_str));
            }
        }
        seen
    }

    /// `id` and every node it descends from; empty for an id the DAG does not
    /// hold. Twin of `Dag.ancestorsOf`.
    pub fn ancestors_of(&self, id: &str) -> BTreeSet<String> {
        self.closure_of([id])
    }

    /// Of a non-empty down-closed set, the member with the largest ancestor
    /// closure, the larger id (ordinal) on a tie (twin of `deepest`). Over an
    /// acyclic set only a maximal member can win, so only those are sized.
    fn deepest(&self, common: &BTreeSet<String>) -> Option<String> {
        let candidates: BTreeSet<String> = if self.drain(|id| common.contains(id)).1.is_empty() {
            let parents: Vec<&str> = common
                .iter()
                .filter_map(|id| self.get(id))
                .flat_map(|n| n.parents.iter().map(String::as_str))
                .collect();
            let below = self.closure_of(parents);
            common.difference(&below).cloned().collect()
        } else {
            common.clone()
        };
        candidates
            .into_iter()
            .map(|id| (self.ancestors_of(&id).len(), id))
            .max_by(|(na, a), (nb, b)| na.cmp(nb).then_with(|| ordinal_cmp(a, b)))
            .map(|(_, id)| id)
    }

    /// The common base of N heads: of the nodes in every head's ancestor closure,
    /// the one with the largest closure, the larger id on a tie (twin of
    /// `Dag.commonBase`). `None` for no heads, or when nothing is common to all. A
    /// function of the head set; a head the DAG does not hold makes it `None`.
    pub fn common_base(&self, heads: &[String]) -> Option<String> {
        let distinct = dedup(heads);
        let (first, rest) = distinct.split_first()?;
        let mut common = self.ancestors_of(first);
        for h in rest {
            let c = self.ancestors_of(h);
            common.retain(|id| c.contains(id));
        }
        if common.is_empty() {
            None
        } else {
            self.deepest(&common)
        }
    }

    /// Apply the ops of `order` (ids the DAG holds) from `state0`; the first
    /// refusal is named by its node id. Twin of `fold`.
    pub(crate) fn fold<S, R>(
        &self,
        state0: S,
        order: &[String],
        apply: &mut impl FnMut(&Op, S) -> Result<S, R>,
    ) -> Result<S, (String, R)> {
        let mut state = state0;
        for id in order {
            state = apply(&self.nodes[id].op, state).map_err(|r| (id.clone(), r))?;
        }
        Ok(state)
    }

    /// Replay `head`'s history from `state0`: its closure in drain order, each op
    /// applied once; the first node the domain refuses is named.
    pub fn replay_to<S, R>(
        &self,
        state0: S,
        head: &str,
        mut apply: impl FnMut(&Op, S) -> Result<S, R>,
    ) -> Result<S, ReplayFault<R>> {
        if !self.nodes.contains_key(head) {
            return Err(ReplayFault::UnknownHead(head.to_string()));
        }
        let closure = self.ancestors_of(head);
        let order =
            Self::ordered(self.drain(|id| closure.contains(id))).map_err(ReplayFault::Cyclic)?;
        self.fold(state0, &order, &mut apply)
            .map_err(|(id, r)| ReplayFault::Rejected(id, r))
    }

    /// Replay the whole DAG from `state0` in [`Dag::total_order`].
    pub fn replay_all<S, R>(
        &self,
        state0: S,
        mut apply: impl FnMut(&Op, S) -> Result<S, R>,
    ) -> Result<S, ReplayAllFault<R>> {
        let order = self.total_order().map_err(ReplayAllFault::Cyclic)?;
        self.fold(state0, &order, &mut apply)
            .map_err(|(id, r)| ReplayAllFault::Rejected(id, r))
    }

    /// The first integrity fault, walking the drain order then any node on or
    /// below a cycle in id order: a node whose id is not its content hash, then a
    /// parent the DAG does not hold. `None` for an intact DAG.
    pub fn first_break(&self) -> Option<DagBreak> {
        let (placed, unplaced) = self.drain(|_| true);
        placed.iter().chain(unplaced.iter()).find_map(|id| {
            let n = &self.nodes[id];
            let h = node_id(&n.parents, &n.actor, &n.op.canonical());
            if *id != h {
                return Some(DagBreak {
                    node: id.clone(),
                    reason: DagBreakReason::ContentIdMismatch,
                    expected: h,
                    got: id.clone(),
                });
            }
            n.parents
                .iter()
                .find(|p| !self.nodes.contains_key(*p))
                .map(|missing| DagBreak {
                    node: id.clone(),
                    reason: DagBreakReason::MissingParent,
                    expected: String::new(),
                    got: missing.clone(),
                })
        })
    }
}

/// `ids` with every repeat after the first dropped, in the order given.
pub(crate) fn dedup(ids: &[String]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for h in ids {
        if !out.contains(&h.as_str()) {
            out.push(h);
        }
    }
    out
}

fn sorted(ids: &[String]) -> Vec<&str> {
    let mut v: Vec<&str> = ids.iter().map(String::as_str).collect();
    v.sort_by(|a, b| ordinal_cmp(a, b));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collision_names_the_node_as_the_reference_does() {
        assert_eq!(
            AppendFault::ContentIdCollision("n1".into()).encode(),
            "{\"$type\":\"contentIdCollision\",\"nodeId\":\"n1\"}"
        );
    }
}
