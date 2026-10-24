//! Change propagation over a declared dependency graph: the evaluation order and
//! its cycles, the minimal dirty set of a change, and the incremental evaluation
//! driver that recomputes only what a change reaches.
//!
//! A dependency map takes each node's id to the ids it reads. The graph questions
//! are answered over a prepared relation (the ids ascending in
//! ordinal order, each node's reads ascending), so every answer is a function of
//! the edge set alone, exactly as the reference's are:
//!
//! - [`sort`] — Tarjan's strongly-connected-components pass, dependencies first;
//!   a node that reads itself, or any group of more than one node, is a cycle,
//!   reported as data and never diverged on;
//! - [`dirty_from_changed_ids`] / [`needed_for`] — the closure of a change over
//!   the inverse relation, and of a target set over the relation itself;
//! - [`eval`] / [`eval_from`] / [`eval_for`] and their prior-aware `_with` forms
//!   — one walk of the acyclic nodes in that order. The spine owns no evaluator:
//!   the domain's evaluator is handed a resolver that answers for the node's
//!   DECLARED reads and nothing else, and a read outside them ends the evaluation
//!   with [`PropagationError::EvalUndeclaredRead`].
//!
//! Twin of `Propagation` (`Fuaran.Core.Propagation`) and the parts of
//! `Fuaran.Core.Relation` it asks; certified against Core's
//! `laws/propagation-laws.json`.

use crate::json::ordinal_cmp;
use crate::prelude::*;

/// A dependency map: each node's id to the ids it reads. A read of an id that is
/// not a key is a dangling read: it is ignored for ordering and is never a cycle.
pub type Deps = BTreeMap<String, BTreeSet<String>>;

// ─── The prepared relation ───────────────────────────────────────────────────

/// A relation prepared for the graph questions: its nodes ascending in ordinal
/// order, and each node's successors as ascending, distinct indices. Twin of
/// `Relation<string>` (`Fuaran.Core.Relation`).
struct Relation {
    nodes: Vec<String>,
    successors: Vec<Vec<usize>>,
}

fn index_of(nodes: &[String], node: &str) -> Option<usize> {
    nodes.binary_search_by(|n| ordinal_cmp(n, node)).ok()
}

impl Relation {
    /// Twin of `Relation.ofMap`: the keys and every successor are its nodes.
    fn of_map<'a, I, S>(edges: I) -> Relation
    where
        I: Iterator<Item = (&'a String, S)> + Clone,
        S: IntoIterator<Item = &'a String>,
    {
        let mut nodes: Vec<String> = Vec::new();
        for (k, reads) in edges.clone() {
            nodes.push(k.clone());
            nodes.extend(reads.into_iter().cloned());
        }
        nodes.sort_by(|a, b| ordinal_cmp(a, b));
        nodes.dedup();
        let mut successors = vec![Vec::new(); nodes.len()];
        for (k, reads) in edges {
            if let Some(i) = index_of(&nodes, k) {
                let mut s: Vec<usize> = reads
                    .into_iter()
                    .filter_map(|r| index_of(&nodes, r))
                    .collect();
                s.sort_unstable();
                s.dedup();
                successors[i] = s;
            }
        }
        Relation { nodes, successors }
    }

    /// Twin of `Relation.inverse`: the same nodes, every edge reversed; each
    /// inverted list ascending and distinct by construction.
    fn inverse(&self) -> Relation {
        let mut inverted = vec![Vec::new(); self.nodes.len()];
        for (v, succ) in self.successors.iter().enumerate() {
            for &w in succ {
                inverted[w].push(v);
            }
        }
        Relation {
            nodes: self.nodes.clone(),
            successors: inverted,
        }
    }

    /// Twin of `Relation.reachable`: every node reachable from a source by zero
    /// edges or more, a source that is no node of the relation included as itself.
    fn reachable<'a>(&self, sources: impl IntoIterator<Item = &'a String>) -> BTreeSet<String> {
        let mut seen = vec![false; self.nodes.len()];
        let mut frontier: Vec<usize> = Vec::new();
        let mut out = BTreeSet::new();
        for source in sources {
            match index_of(&self.nodes, source) {
                None => {
                    out.insert(source.clone());
                }
                Some(i) if !seen[i] => {
                    seen[i] = true;
                    frontier.push(i);
                }
                Some(_) => {}
            }
        }
        while let Some(v) = frontier.pop() {
            for &w in &self.successors[v] {
                if !seen[w] {
                    seen[w] = true;
                    frontier.push(w);
                }
            }
        }
        for (i, node) in self.nodes.iter().enumerate() {
            if seen[i] {
                out.insert(node.clone());
            }
        }
        out
    }

    /// Tarjan's pass, iterative (twin of `Relation.pass`): roots ascending, each
    /// node's successors ascending, a child's low-link folded into its parent's when
    /// the child's frame finishes, a component emitted when its root's frame does.
    /// The components come in the order the pass finished them; each component's
    /// members in the order Tarjan's stack popped them.
    fn components(&self) -> Vec<Vec<usize>> {
        let n = self.nodes.len();
        let mut order: Vec<Option<usize>> = vec![None; n];
        let mut low = vec![0usize; n];
        let mut on_stack = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut frames: Vec<usize> = Vec::new();
        let mut frame_next = vec![0usize; n];
        let mut next = 0usize;
        let mut components = Vec::new();

        for root in 0..n {
            if order[root].is_some() {
                continue;
            }
            enter(
                root,
                &mut order,
                &mut low,
                &mut next,
                &mut stack,
                &mut on_stack,
                &mut frames,
            );
            while let Some(&v) = frames.last() {
                let succ = &self.successors[v];
                let i = frame_next[v];
                if i < succ.len() {
                    frame_next[v] = i + 1;
                    let w = succ[i];
                    match order[w] {
                        None => enter(
                            w,
                            &mut order,
                            &mut low,
                            &mut next,
                            &mut stack,
                            &mut on_stack,
                            &mut frames,
                        ),
                        Some(ow) if on_stack[w] => low[v] = low[v].min(ow),
                        Some(_) => {}
                    }
                } else {
                    frames.pop();
                    if Some(low[v]) == order[v] {
                        let mut members = Vec::new();
                        while let Some(w) = stack.pop() {
                            on_stack[w] = false;
                            members.push(w);
                            if w == v {
                                break;
                            }
                        }
                        components.push(members);
                    }
                    if let Some(&u) = frames.last() {
                        low[u] = low[u].min(low[v]);
                    }
                }
            }
        }
        components
    }

    /// More than one member, or one member with an edge to itself.
    fn is_cycle(&self, members: &[usize]) -> bool {
        members.len() > 1 || self.successors[members[0]].contains(&members[0])
    }

    fn lift(&self, members: &[usize]) -> Vec<String> {
        members.iter().map(|&i| self.nodes[i].clone()).collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn enter(
    v: usize,
    order: &mut [Option<usize>],
    low: &mut [usize],
    next: &mut usize,
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
    frames: &mut Vec<usize>,
) {
    order[v] = Some(*next);
    low[v] = *next;
    *next += 1;
    stack.push(v);
    on_stack[v] = true;
    frames.push(v);
}

fn ordinal_sorted(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = ids.into_iter().collect();
    v.sort_by(|a, b| ordinal_cmp(a, b));
    v
}

// ─── The order and the closures ──────────────────────────────────────────────

/// The evaluation order of a dependency map and its cycles. Twin of
/// `Propagation.TopoResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopoResult {
    /// The acyclic ids, each once, every one after each acyclic id it reads; ids
    /// in a cycle and dangling reads never appear here.
    pub order: Vec<String>,
    /// One list per cyclic group, the groups in dependencies-first order; the ids
    /// within a group in Tarjan's stack order, not sorted.
    pub cycles: Vec<Vec<String>>,
}

/// The dependencies-first order of `deps` and its cyclic groups: Tarjan's pass
/// over the map with its dangling reads removed, the ids visited ascending in
/// ordinal order with their reads ascending. Total — a cycle is data. Twin of
/// `Propagation.sort` (and `Graph.sort`).
pub fn sort(deps: &Deps) -> TopoResult {
    let known: Vec<(&String, Vec<&String>)> = deps
        .iter()
        .map(|(k, reads)| (k, reads.iter().filter(|r| deps.contains_key(*r)).collect()))
        .collect();
    let relation = Relation::of_map(known.iter().map(|(k, r)| (*k, r.iter().copied())));
    let mut order = Vec::new();
    let mut cycles = Vec::new();
    for members in relation.components() {
        if relation.is_cycle(&members) {
            cycles.push(relation.lift(&members));
        } else {
            order.push(relation.nodes[members[0]].clone());
        }
    }
    TopoResult { order, cycles }
}

/// Whether `topo` is a valid evaluation order of `deps`: no id twice; the order
/// and the cycles' members partition the keys; every read of an ordered id that
/// `deps` holds lies earlier in the order or in a cycle. A checker, not a sort.
/// Twin of `Propagation.validTopo`.
pub fn valid_topo(deps: &Deps, topo: &TopoResult) -> bool {
    let cyclic: Vec<&String> = topo.cycles.iter().flatten().collect();
    let all: Vec<&String> = topo.order.iter().chain(cyclic.iter().copied()).collect();
    let distinct: BTreeSet<&String> = all.iter().copied().collect();
    if distinct.len() != all.len()
        || distinct.len() != deps.len()
        || !distinct.iter().all(|k| deps.contains_key(*k))
    {
        return false;
    }
    let in_cycle: BTreeSet<&String> = cyclic.into_iter().collect();
    let mut seen: BTreeSet<&String> = BTreeSet::new();
    for id in &topo.order {
        if let Some(reads) = deps.get(id) {
            let ok = reads
                .iter()
                .all(|r| !deps.contains_key(r) || seen.contains(r) || in_cycle.contains(r));
            if !ok {
                return false;
            }
        }
        seen.insert(id);
    }
    true
}

/// The cyclic group through `target`, if any. Twin of `Propagation.cycleThrough`.
pub fn cycle_through(target: &str, deps: &Deps) -> Option<Vec<String>> {
    sort(deps)
        .cycles
        .into_iter()
        .find(|c| c.iter().any(|id| id == target))
}

/// The dependents map: each id something reads to the ids that read it; an id
/// nothing reads has no key. Twin of `Propagation.dependents` (and
/// `Graph.dependents`).
pub fn dependents(deps: &Deps) -> Deps {
    let inv = Relation::of_map(deps.iter()).inverse();
    let mut out = Deps::new();
    for (i, node) in inv.nodes.iter().enumerate() {
        if !inv.successors[i].is_empty() {
            out.insert(
                node.clone(),
                inv.lift(&inv.successors[i]).into_iter().collect(),
            );
        }
    }
    out
}

/// The minimal dirty set of a change: `changed` and every id that transitively
/// reads one. Twin of `Propagation.dirtyFromChangedIds`.
pub fn dirty_from_changed_ids(deps: &Deps, changed: &BTreeSet<String>) -> BTreeSet<String> {
    Relation::of_map(deps.iter()).inverse().reachable(changed)
}

/// The needed set of a target set: the targets and every id they transitively
/// read. Twin of `Propagation.neededFor`.
pub fn needed_for(deps: &Deps, targets: &BTreeSet<String>) -> BTreeSet<String> {
    Relation::of_map(deps.iter()).reachable(targets)
}

// ─── The incremental driver ──────────────────────────────────────────────────

/// Why an evaluation failed — named, never a panic. A cycle is not a failure: it
/// is data in [`EvalOutcome::cyclic`]. Twin of `Propagation.PropagationError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropagationError {
    /// Some changed ids are not keys of the map: every such id, in ascending
    /// ordinal order. Raised before any node is evaluated.
    EvalUnknownChange(Vec<String>),
    /// The evaluator returned `Err(message)` at `node`; the walk stops there.
    EvalNodeFailed { node: String, message: String },
    /// The evaluator at `node` asked for `read`, outside the node's declared
    /// reads; only the first such read is named, and it outranks an `Err` the
    /// evaluator returned.
    EvalUndeclaredRead { node: String, read: String },
}

/// The outcome of an evaluation: one value per evaluated acyclic node, and the
/// cyclic groups, never evaluated. Twin of `Propagation.EvalOutcome`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalOutcome<V> {
    pub values: BTreeMap<String, V>,
    pub cyclic: Vec<Vec<String>>,
}

/// A dependency map prepared for repeated evaluation: its order and cycles and
/// its dependents, derived once. Twin of `Propagation.Plan`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub deps: Deps,
    pub topo: TopoResult,
    pub dependents: Deps,
}

/// Prepare `deps`: one [`sort`], one [`dependents`]. Twin of `Propagation.plan`.
pub fn plan(deps: &Deps) -> Plan {
    Plan {
        deps: deps.clone(),
        topo: sort(deps),
        dependents: dependents(deps),
    }
}

/// What an evaluation computes. Twin of `Propagation.Scope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope<V> {
    /// Every acyclic node, once, in dependency order.
    Full,
    /// Recompute the dirty set of `changed`, reusing each clean node's `prior`
    /// value; a changed id the map does not hold is `EvalUnknownChange`.
    From {
        prior: BTreeMap<String, V>,
        changed: BTreeSet<String>,
    },
    /// The targets and what they read, and nothing else.
    For { targets: BTreeSet<String> },
}

/// The resolver an evaluator reads its declared reads through: a computed value,
/// or `None` for a cyclic, dangling, failed or not-yet-reached read.
pub type Resolve<'a, V> = dyn FnMut(&str) -> Option<V> + 'a;

/// The shared walk (twin of `Propagation.walkTopo`): the order's nodes in turn,
/// each recomputed when `recompute` says so or `prior` lacks it, else reused.
fn walk_topo<V, F>(
    eval_node: &mut F,
    recompute: &dyn Fn(&str) -> bool,
    prior: &BTreeMap<String, V>,
    deps: &Deps,
    order: &[String],
    cycles: &[Vec<String>],
) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, Option<&V>, &str) -> Result<V, String>,
{
    let empty = BTreeSet::new();
    let mut results: BTreeMap<String, V> = BTreeMap::new();
    for id in order {
        match prior.get(id) {
            Some(p) if !recompute(id) => {
                results.insert(id.clone(), p.clone());
            }
            own_prior => {
                let declared = deps.get(id).unwrap_or(&empty);
                let mut undeclared: Option<String> = None;
                let computed = {
                    let mut resolve = |k: &str| {
                        if declared.contains(k) {
                            results.get(k).cloned()
                        } else {
                            if undeclared.is_none() {
                                undeclared = Some(k.to_string());
                            }
                            None
                        }
                    };
                    eval_node(&mut resolve, own_prior, id)
                };
                if let Some(read) = undeclared {
                    return Err(PropagationError::EvalUndeclaredRead {
                        node: id.clone(),
                        read,
                    });
                }
                match computed {
                    Ok(v) => {
                        results.insert(id.clone(), v);
                    }
                    Err(message) => {
                        return Err(PropagationError::EvalNodeFailed {
                            node: id.clone(),
                            message,
                        });
                    }
                }
            }
        }
    }
    Ok(EvalOutcome {
        values: results,
        cyclic: cycles.to_vec(),
    })
}

/// The one evaluator body (twin of `Propagation.scoped`).
fn scoped<V, F>(
    eval_node: &mut F,
    scope: &Scope<V>,
    deps: &Deps,
    topo: &TopoResult,
    dependents_of: &dyn Fn() -> Deps,
) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, Option<&V>, &str) -> Result<V, String>,
{
    match scope {
        Scope::Full => walk_topo(
            eval_node,
            &|_| true,
            &BTreeMap::new(),
            deps,
            &topo.order,
            &topo.cycles,
        ),
        Scope::From { prior, changed } => {
            let unknown: Vec<String> = changed
                .iter()
                .filter(|c| !deps.contains_key(*c))
                .cloned()
                .collect();
            if !unknown.is_empty() {
                return Err(PropagationError::EvalUnknownChange(ordinal_sorted(unknown)));
            }
            let dirty = Relation::of_map(dependents_of().iter()).reachable(changed);
            walk_topo(
                eval_node,
                &|id| dirty.contains(id),
                prior,
                deps,
                &topo.order,
                &topo.cycles,
            )
        }
        Scope::For { targets } => {
            let needed = needed_for(deps, targets);
            let order: Vec<String> = topo
                .order
                .iter()
                .filter(|id| needed.contains(*id))
                .cloned()
                .collect();
            let cycles: Vec<Vec<String>> = topo
                .cycles
                .iter()
                .filter(|c| c.iter().any(|id| needed.contains(id)))
                .cloned()
                .collect();
            walk_topo(
                eval_node,
                &|_| true,
                &BTreeMap::new(),
                deps,
                &order,
                &cycles,
            )
        }
    }
}

/// Evaluate a scope over a prepared plan, each recomputed node handed its own
/// prior value. Twin of `Propagation.evalScopedWith`.
pub fn eval_scoped_with<V, F>(
    mut eval_node: F,
    scope: &Scope<V>,
    p: &Plan,
) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, Option<&V>, &str) -> Result<V, String>,
{
    scoped(&mut eval_node, scope, &p.deps, &p.topo, &|| {
        p.dependents.clone()
    })
}

/// [`eval_scoped_with`] over an evaluator that ignores its prior. Twin of
/// `Propagation.evalScoped`.
pub fn eval_scoped<V, F>(
    mut eval_node: F,
    scope: &Scope<V>,
    p: &Plan,
) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, &str) -> Result<V, String>,
{
    eval_scoped_with(
        |r: &mut Resolve<'_, V>, _: Option<&V>, id: &str| eval_node(r, id),
        scope,
        p,
    )
}

/// The full evaluator over a prior-aware evaluator, every node handed `None` as
/// its prior. Twin of `Propagation.evalWith`.
pub fn eval_with<V, F>(mut eval_node: F, deps: &Deps) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, Option<&V>, &str) -> Result<V, String>,
{
    scoped(&mut eval_node, &Scope::Full, deps, &sort(deps), &|| {
        dependents(deps)
    })
}

/// The reference full evaluator: every acyclic node once, in dependency order.
/// Twin of `Propagation.eval`.
pub fn eval<V, F>(mut eval_node: F, deps: &Deps) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, &str) -> Result<V, String>,
{
    eval_with(
        |r: &mut Resolve<'_, V>, _: Option<&V>, id: &str| eval_node(r, id),
        deps,
    )
}

/// Incremental re-evaluation over a prior-aware evaluator: the dirty set of
/// `changed` recomputed, each clean node's prior reused. Twin of
/// `Propagation.evalFromWith`.
pub fn eval_from_with<V, F>(
    eval_node: F,
    prior: &BTreeMap<String, V>,
    changed: &BTreeSet<String>,
    deps: &Deps,
) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, Option<&V>, &str) -> Result<V, String>,
{
    let scope = Scope::From {
        prior: prior.clone(),
        changed: changed.clone(),
    };
    eval_scoped_with(eval_node, &scope, &plan(deps))
}

/// Incremental re-evaluation: byte-identical to a full [`eval`] when `changed`
/// names every node whose evaluation differs and `prior` came from an
/// evaluation over the same map. Twin of `Propagation.evalFrom`.
pub fn eval_from<V, F>(
    mut eval_node: F,
    prior: &BTreeMap<String, V>,
    changed: &BTreeSet<String>,
    deps: &Deps,
) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, &str) -> Result<V, String>,
{
    eval_from_with(
        |r: &mut Resolve<'_, V>, _: Option<&V>, id: &str| eval_node(r, id),
        prior,
        changed,
        deps,
    )
}

/// Demand-driven evaluation: the targets and what they read, once each in
/// dependency order, and nothing else. Twin of `Propagation.evalFor`.
pub fn eval_for<V, F>(
    mut eval_node: F,
    targets: &BTreeSet<String>,
    deps: &Deps,
) -> Result<EvalOutcome<V>, PropagationError>
where
    V: Clone,
    F: FnMut(&mut Resolve<'_, V>, &str) -> Result<V, String>,
{
    let scope = Scope::For {
        targets: targets.clone(),
    };
    scoped(
        &mut |r: &mut Resolve<'_, V>, _: Option<&V>, id: &str| eval_node(r, id),
        &scope,
        deps,
        &sort(deps),
        &|| dependents(deps),
    )
}
