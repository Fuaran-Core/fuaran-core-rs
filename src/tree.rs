//! The skeleton tree — the witness surface Core's structural ops read: a node is
//! an id, a kind and its children, and nothing else. A host with a richer node
//! type runs the ops by mapping those three members onto its own node.
//!
//! Its canonical bytes are `{"children":[…],"id":…,"kind":…}` (members sorted
//! ordinally), the envelope Core's `apply/`, `dag/`, `merge/` and `diff/`
//! families carry every tree in.

use crate::decode::{Cursor, DecodeError};
use crate::json::escape_string;
use crate::prelude::*;

/// A skeleton node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub children: Vec<Node>,
}

impl Node {
    /// A node with no children.
    pub fn leaf(id: impl Into<String>, kind: impl Into<String>) -> Node {
        Node {
            id: id.into(),
            kind: kind.into(),
            children: Vec::new(),
        }
    }

    /// The node with this id, anywhere in the tree (this node included).
    pub fn find(&self, id: &str) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    pub(crate) fn find_mut(&mut self, id: &str) -> Option<&mut Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_mut(id))
    }

    /// Every id in the tree, in pre-order.
    pub fn ids(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_ids(&mut out);
        out
    }

    fn collect_ids<'a>(&'a self, out: &mut Vec<&'a str>) {
        out.push(&self.id);
        for c in &self.children {
            c.collect_ids(out);
        }
    }

    /// The node's canonical bytes.
    pub fn encode(&self) -> String {
        let mut out = String::from("{\"children\":[");
        for (i, c) in self.children.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&c.encode());
        }
        out.push_str("],\"id\":");
        out.push_str(&escape_string(&self.id));
        out.push_str(",\"kind\":");
        out.push_str(&escape_string(&self.kind));
        out.push('}');
        out
    }

    /// Read a node: string `id` and `kind` members and a `children` array of
    /// nodes, each refusal at its path. Any other member is ignored.
    pub fn decode(at: &Cursor<'_>) -> Result<Node, DecodeError> {
        let id = at.field("id")?.string()?.to_string();
        let kind = at.field("kind")?.string()?.to_string();
        let children = at
            .field("children")?
            .items()?
            .iter()
            .map(Node::decode)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Node { id, kind, children })
    }
}
