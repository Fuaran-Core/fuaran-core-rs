//! Branch reconciliation over the op-DAG: the merge-conflict report between two
//! deltas, and the N-lane reconcile that folds them into one script or names why
//! it cannot.
//!
//! - [`conflicts`] enumerates the op pairs of two deltas whose footprints
//!   ([`crate::ops::footprint`]) are not independent (twin of `Dag.conflicts`);
//! - [`Dag::common_base`] is the deepest node every head descends from (twin
//!   of `Dag.commonBase`);
//! - [`reconcile_many`] partitions the region above a base into shared history
//!   and one exclusive delta per head, checks every unordered pair of exclusive
//!   deltas, replays the shared region once and each lane on its own, and answers
//!   the script `shared ++ lane_1 ++ … ++ lane_n` or a [`ReconcileFault`] (twin of
//!   `Dag.reconcileMany`).
//!
//! Every answer is a function of the head SET up to the script's canonical lane
//! order: the interference report is symmetric up to a left/right swap, and the
//! rejecting lanes are sorted by head id. Certified against Core's
//! `merge/skeleton-merge.json`.

use crate::dag::{Dag, DagOp, dedup};
use crate::json::ordinal_cmp;
use crate::ops::Footprint;
use crate::prelude::*;

// ─── the conflict report ─────────────────────────────────────────────────────

/// How two ops collide at an address (twin of `MergeConflictShape`, less its
/// slot case, which no skeleton footprint can raise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeConflictShape {
    /// Both touch the same node's content, at least one writing it.
    ConcurrentUpdate,
    /// Both write the child list of the same named parent.
    InsertPositionClash,
    /// A removal, move or update races a structural write on the other side.
    MoveVsRemove,
}

impl MergeConflictShape {
    /// The shape's canonical spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            MergeConflictShape::ConcurrentUpdate => "concurrentUpdate",
            MergeConflictShape::InsertPositionClash => "insertPositionClash",
            MergeConflictShape::MoveVsRemove => "moveVsRemove",
        }
    }
}

/// One enumerated interference (twin of `MergeConflict`): the op from the first
/// delta, the op from the second, the address they collide on and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflict<O> {
    pub left: O,
    pub right: O,
    pub address: String,
    pub shape: MergeConflictShape,
}

fn sorted_ordinal(set: BTreeSet<&String>) -> Vec<String> {
    let mut v: Vec<String> = set.into_iter().cloned().collect();
    v.sort_by(|a, b| ordinal_cmp(a, b));
    v
}

/// Every op pair `(a ∈ delta_a, b ∈ delta_b)` whose footprints are not
/// independent, one conflict per shared address, tagged by the most specific
/// shape (content, then position, then move/remove) (twin of `Dag.conflicts`).
/// Ordered `delta_a` × `delta_b` × ordinal address. `[]` promises the two deltas
/// commute. Detection only: nothing is applied.
pub fn conflicts<O: Clone>(
    footprint_of: &impl Fn(&O) -> Footprint,
    delta_a: &[O],
    delta_b: &[O],
) -> Vec<MergeConflict<O>> {
    let mut out = Vec::new();
    for a in delta_a {
        let fa = footprint_of(a);
        for b in delta_b {
            let fb = footprint_of(b);
            let mut c1: BTreeSet<&String> = BTreeSet::new();
            c1.extend(fa.content_writes.intersection(&fb.content_writes));
            c1.extend(fa.content_writes.intersection(&fb.reads));
            c1.extend(fb.content_writes.intersection(&fa.reads));
            let c2: BTreeSet<&String> = fa
                .structure_writes
                .intersection(&fb.structure_writes)
                .filter(|x| !c1.contains(x))
                .collect();
            let mut move_remove: BTreeSet<&String> = BTreeSet::new();
            if !fa.unknown_parent_writes.is_empty() && fb.writes_structure() {
                move_remove.extend(fa.unknown_parent_writes.iter());
            }
            if !fb.unknown_parent_writes.is_empty() && fa.writes_structure() {
                move_remove.extend(fb.unknown_parent_writes.iter());
            }
            let c3: BTreeSet<&String> = move_remove
                .into_iter()
                .filter(|x| !c1.contains(x) && !c2.contains(x))
                .collect();
            for (set, shape) in [
                (c1, MergeConflictShape::ConcurrentUpdate),
                (c2, MergeConflictShape::InsertPositionClash),
                (c3, MergeConflictShape::MoveVsRemove),
            ] {
                for address in sorted_ordinal(set) {
                    out.push(MergeConflict {
                        left: a.clone(),
                        right: b.clone(),
                        address,
                        shape,
                    });
                }
            }
        }
    }
    out
}

// ─── the N-lane reconcile ────────────────────────────────────────────────────

/// One lane the reconcile refused because its own delta does not apply from the
/// shared state (twin of `LaneRejection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneRejection<O, R> {
    /// The lane's head id.
    pub head: String,
    /// Every op of the lane's exclusive delta, in replay order.
    pub delta: Vec<O>,
    /// The first node of the delta the domain refused.
    pub node_id: String,
    /// The domain's rejection of that node's op.
    pub reject: R,
}

/// Why a lane set did not fold (twin of `ReconcileFault`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileFault<O, R> {
    /// Two lanes' exclusive deltas interfere: every conflict over every unordered
    /// lane pair, in the order the heads were named; nothing applied.
    LanesInterfere(Vec<MergeConflict<O>>),
    /// The history two or more heads share above the base does not replay from
    /// the base's state: the node and the rejection.
    SharedHistoryRejected(String, R),
    /// Lanes that do not apply on their own from the shared state, sorted by head
    /// id (ordinal).
    LanesRejected(Vec<LaneRejection<O, R>>),
}

/// The region above a base: the shared ids and each distinct head's exclusive
/// ids, both in the drain order of the heads' union.
struct Region {
    shared: Vec<String>,
    exclusive: Vec<(String, Vec<String>)>,
}

fn region<O: DagOp>(dag: &Dag<O>, base: &str, heads: &[String]) -> Region {
    let hs = dedup(heads);
    let base_closure = dag.ancestors_of(base);
    let closures: Vec<(&str, BTreeSet<String>)> =
        hs.iter().map(|h| (*h, dag.ancestors_of(h))).collect();
    let owners = |id: &str| closures.iter().filter(|(_, c)| c.contains(id)).count();
    let union = dag.closure_of(hs.iter().copied());
    let above: Vec<String> = dag
        .drain(|id| union.contains(id))
        .0
        .into_iter()
        .filter(|id| !base_closure.contains(id))
        .collect();
    Region {
        shared: above.iter().filter(|id| owners(id) >= 2).cloned().collect(),
        exclusive: closures
            .iter()
            .map(|(h, c)| {
                let ids = above
                    .iter()
                    .filter(|id| c.contains(*id) && owners(id) == 1)
                    .cloned()
                    .collect();
                (h.to_string(), ids)
            })
            .collect(),
    }
}

fn ops_of<O: DagOp + Clone>(dag: &Dag<O>, ids: &[String]) -> Vec<O> {
    ids.iter()
        .filter_map(|id| dag.get(id).map(|n| n.op.clone()))
        .collect()
}

/// Fold `heads`, N branch heads over `base` whose state is `base_state`, into
/// one merge script (twin of `Dag.reconcileMany`):
///
/// 1. the region above the base is partitioned: heads deduplicated (first
///    occurrence kept), history two or more heads hold is SHARED, the rest of
///    each head's closure above the base its EXCLUSIVE delta;
/// 2. every unordered pair of exclusive deltas is checked with [`conflicts`];
///    any interference is [`ReconcileFault::LanesInterfere`];
/// 3. the shared region is replayed from `base_state`; a rejection is
///    [`ReconcileFault::SharedHistoryRejected`];
/// 4. each exclusive delta is replayed on its own from that state; every lane
///    that rejects is reported, sorted by head id, as
///    [`ReconcileFault::LanesRejected`];
/// 5. otherwise the script `shared ++ exclusive_1 ++ … ++ exclusive_n`, the lanes
///    in the order `heads` names them.
pub fn reconcile_many<O, S, R>(
    dag: &Dag<O>,
    footprint_of: impl Fn(&O) -> Footprint,
    base: &str,
    base_state: S,
    heads: &[String],
    mut apply: impl FnMut(&O, S) -> Result<S, R>,
) -> Result<Vec<O>, ReconcileFault<O, R>>
where
    O: DagOp + Clone,
    S: Clone,
{
    let r = region(dag, base, heads);
    let deltas: Vec<(&str, &[String], Vec<O>)> = r
        .exclusive
        .iter()
        .map(|(h, ids)| (h.as_str(), ids.as_slice(), ops_of(dag, ids)))
        .collect();
    let mut interfering = Vec::new();
    for (i, (_, _, a)) in deltas.iter().enumerate() {
        for (_, _, b) in &deltas[i + 1..] {
            interfering.extend(conflicts(&footprint_of, a, b));
        }
    }
    if !interfering.is_empty() {
        return Err(ReconcileFault::LanesInterfere(interfering));
    }
    let shared_state = dag
        .fold(base_state, &r.shared, &mut apply)
        .map_err(|(n, rej)| ReconcileFault::SharedHistoryRejected(n, rej))?;
    let mut rejected: Vec<LaneRejection<O, R>> = Vec::new();
    for (h, ids, ops) in &deltas {
        if let Err((node_id, reject)) = dag.fold(shared_state.clone(), ids, &mut apply) {
            rejected.push(LaneRejection {
                head: h.to_string(),
                delta: ops.clone(),
                node_id,
                reject,
            });
        }
    }
    if !rejected.is_empty() {
        rejected.sort_by(|a, b| ordinal_cmp(&a.head, &b.head));
        return Err(ReconcileFault::LanesRejected(rejected));
    }
    let mut script = ops_of(dag, &r.shared);
    for (_, _, ops) in deltas {
        script.extend(ops);
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Op, footprint};
    use crate::tree::Node;

    #[test]
    fn an_update_is_dependent_on_any_structural_write() {
        let up = Op::UpdateNode {
            node: Node::leaf("a1", "quote"),
        };
        let ins = Op::InsertChild {
            parent: "b".into(),
            node: Node::leaf("y", "para"),
        };
        let cs = conflicts(&|o: &Op| footprint(core::slice::from_ref(o)), &[up], &[ins]);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].address, "a1");
        assert_eq!(cs[0].shape, MergeConflictShape::MoveVsRemove);
    }
}
