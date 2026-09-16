//! Isolated persistent compressed byte-radix prototype, not a production index.
#![forbid(unsafe_code)]

use std::fmt;
use std::iter::FusedIterator;
use std::ops::{Bound, RangeBounds};
use std::sync::Arc;

#[derive(Clone)]
struct Entry {
    key: Vec<u8>,
    value: Vec<u8>,
}

#[derive(Clone)]
struct Edge {
    byte: u8,
    node: Arc<Node>,
}

/// Prefix bytes are relative to the incoming edge, not the root. A terminal
/// entry represents exactly the accumulated path. Edges are sorted and unique.
/// Every published branch has at least two alternatives (terminal + edges).
#[derive(Clone)]
struct Branch {
    prefix: Vec<u8>,
    terminal: Option<Entry>,
    edges: Vec<Edge>,
}

// Box the less common branch so every leaf does not carry a branch-sized union.
// Leaves keep the original, separate Vec key/value allocations. Sorted compact
// edge vectors support all 256 byte labels without a 256-pointer leaf footprint.
#[derive(Clone)]
enum Node {
    Leaf(Entry),
    Branch(Box<Branch>),
}

impl Drop for Node {
    fn drop(&mut self) {
        // Node-level teardown also protects temporary paths and replacement,
        // not just the public root. Empty detached nodes have constant-depth
        // destructors. into_inner handles concurrent last-owner release.
        let mut pending = match self {
            Self::Leaf(_) => return,
            Self::Branch(branch) => std::mem::take(&mut branch.edges),
        };
        while let Some(edge) = pending.pop() {
            if let Some(Self::Branch(branch)) = Arc::into_inner(edge.node).as_mut() {
                pending.append(&mut branch.edges);
            }
        }
    }
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn branch_mut(node: &mut Arc<Node>) -> &mut Branch {
    match Arc::make_mut(node) {
        Node::Branch(branch) => branch,
        Node::Leaf(_) => unreachable!("branch action selected from the same node"),
    }
}

fn leaf(entry: Entry) -> Arc<Node> {
    Arc::new(Node::Leaf(entry))
}

fn branch(prefix: Vec<u8>, terminal: Option<Entry>, mut edges: Vec<Edge>) -> Arc<Node> {
    edges.sort_unstable_by_key(|edge| edge.byte);
    Arc::new(Node::Branch(Box::new(Branch {
        prefix,
        terminal,
        edges,
    })))
}

enum InsertAction {
    ReplaceLeaf,
    SplitLeaf(usize),
    SplitBranch(usize),
    Terminal,
    AddEdge(usize),
    Descend(usize, usize),
}

/// Ordered binary keys with O(1) root clones and copy-on-write mutation paths.
/// All depth-dependent walks, including destruction and Debug, are iterative.
/// This prototype does not promise recovery from allocation failure or panic.
#[derive(Clone, Default)]
pub struct RadixMap {
    root: Option<Arc<Node>>,
    size: usize,
}

impl fmt::Debug for RadixMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RadixMap").field("len", &self.size).finish()
    }
}

impl RadixMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn get(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.get_key_value(key).map(|(_, value)| value)
    }

    pub fn get_key_value(&self, key: &[u8]) -> Option<(&Vec<u8>, &Vec<u8>)> {
        let mut node = self.root.as_deref()?;
        let mut depth = 0;
        loop {
            match node {
                Node::Leaf(entry) => {
                    return (entry.key == key).then_some((&entry.key, &entry.value));
                }
                Node::Branch(branch) => {
                    if !key[depth..].starts_with(&branch.prefix) {
                        return None;
                    }
                    depth += branch.prefix.len();
                    let Some(&byte) = key.get(depth) else {
                        return branch.terminal.as_ref().map(|e| (&e.key, &e.value));
                    };
                    let index = branch.edges.binary_search_by_key(&byte, |e| e.byte).ok()?;
                    node = &branch.edges[index].node;
                    depth += 1;
                }
            }
        }
    }

    /// Returns true for a new key, false for exact replacement.
    pub fn insert_mut(&mut self, key: Vec<u8>, value: Vec<u8>) -> bool {
        let entry = Entry { key, value };
        let Some(mut slot) = self.root.as_mut() else {
            self.root = Some(leaf(entry));
            self.size = 1;
            return true;
        };
        let mut depth = 0;
        let inserted = loop {
            let action = match slot.as_ref() {
                Node::Leaf(old) if old.key == entry.key => InsertAction::ReplaceLeaf,
                Node::Leaf(old) => {
                    InsertAction::SplitLeaf(common_prefix(&old.key[depth..], &entry.key[depth..]))
                }
                Node::Branch(branch) => {
                    let common = common_prefix(&branch.prefix, &entry.key[depth..]);
                    if common < branch.prefix.len() {
                        InsertAction::SplitBranch(common)
                    } else {
                        let end = depth + common;
                        match entry.key.get(end) {
                            None => InsertAction::Terminal,
                            Some(byte) => {
                                match branch.edges.binary_search_by_key(byte, |edge| edge.byte) {
                                    Ok(index) => InsertAction::Descend(index, end + 1),
                                    Err(index) => InsertAction::AddEdge(index),
                                }
                            }
                        }
                    }
                }
            };
            match action {
                InsertAction::ReplaceLeaf => {
                    // Do not clone old key/value buffers just to discard them.
                    if let Some(Node::Leaf(old)) = Arc::get_mut(slot) {
                        *old = entry;
                    } else {
                        *slot = leaf(entry);
                    }
                    break false;
                }
                InsertAction::SplitLeaf(common) => {
                    Self::split_leaf(slot, entry, depth, common);
                    break true;
                }
                InsertAction::SplitBranch(common) => {
                    Self::split_branch(slot, entry, depth, common);
                    break true;
                }
                InsertAction::Terminal => {
                    break branch_mut(slot).terminal.replace(entry).is_none();
                }
                InsertAction::AddEdge(index) => {
                    let branch = branch_mut(slot);
                    let byte = entry.key[depth + branch.prefix.len()];
                    branch.edges.insert(
                        index,
                        Edge {
                            byte,
                            node: leaf(entry),
                        },
                    );
                    break true;
                }
                InsertAction::Descend(index, next_depth) => {
                    slot = &mut branch_mut(slot).edges[index].node;
                    depth = next_depth;
                }
            }
        };
        self.size += usize::from(inserted);
        inserted
    }

    fn split_leaf(slot: &mut Arc<Node>, entry: Entry, depth: usize, common: usize) {
        let cut = depth + common;
        let (prefix, old_byte) = match slot.as_ref() {
            Node::Leaf(old) => (old.key[depth..cut].to_vec(), old.key.get(cut).copied()),
            Node::Branch(_) => unreachable!(),
        };
        let new_byte = entry.key.get(cut).copied();
        match (old_byte, new_byte) {
            (None, Some(byte)) => {
                // The old leaf becomes the branch terminal. Unique buffers move;
                // a shared old leaf is cloned by make_mut before extraction.
                let Node::Leaf(old) = Arc::make_mut(slot) else {
                    unreachable!()
                };
                let terminal = Entry {
                    key: std::mem::take(&mut old.key),
                    value: std::mem::take(&mut old.value),
                };
                *slot = branch(
                    prefix,
                    Some(terminal),
                    vec![Edge {
                        byte,
                        node: leaf(entry),
                    }],
                );
            }
            (Some(byte), None) => {
                let child = Arc::clone(slot);
                *slot = branch(prefix, Some(entry), vec![Edge { byte, node: child }]);
            }
            (Some(old_byte), Some(new_byte)) => {
                let child = Arc::clone(slot);
                *slot = branch(
                    prefix,
                    None,
                    vec![
                        Edge {
                            byte: old_byte,
                            node: child,
                        },
                        Edge {
                            byte: new_byte,
                            node: leaf(entry),
                        },
                    ],
                );
            }
            (None, None) => unreachable!("equal leaves take the replacement path"),
        }
    }

    fn split_branch(slot: &mut Arc<Node>, entry: Entry, depth: usize, common: usize) {
        let old = branch_mut(slot);
        let prefix = old.prefix[..common].to_vec();
        let old_byte = old.prefix[common];
        old.prefix.drain(..=common);
        let child = Arc::clone(slot);
        let mut edges = vec![Edge {
            byte: old_byte,
            node: child,
        }];
        let terminal = if let Some(&byte) = entry.key.get(depth + common) {
            edges.push(Edge {
                byte,
                node: leaf(entry),
            });
            None
        } else {
            Some(entry)
        };
        *slot = branch(prefix, terminal, edges);
    }

    pub fn remove_mut(&mut self, key: &[u8]) -> bool {
        // An absent deletion preserves the root and allocates/clones no path.
        if self.get(key).is_none() {
            return false;
        }
        let mut current = self.root.take().expect("presence implies a root");
        let mut frames = Vec::new();
        let mut depth = 0;
        let mut replacement = loop {
            match current.as_ref() {
                Node::Leaf(_) => break None,
                Node::Branch(branch) => depth += branch.prefix.len(),
            }
            if depth == key.len() {
                branch_mut(&mut current).terminal = None;
                break Self::normalize(current);
            }
            let branch = branch_mut(&mut current);
            let index = branch
                .edges
                .binary_search_by_key(&key[depth], |e| e.byte)
                .expect("presence was checked before mutation");
            let edge = branch.edges.remove(index);
            frames.push((current, index, edge.byte));
            current = edge.node;
            depth += 1;
        };
        // A detached parent has one missing edge; restore or remove it, then
        // collapse locally. No recursion and no second traversal per ancestor.
        while let Some((mut parent, index, byte)) = frames.pop() {
            if let Some(node) = replacement {
                branch_mut(&mut parent)
                    .edges
                    .insert(index, Edge { byte, node });
            }
            replacement = Self::normalize(parent);
        }
        self.root = replacement;
        self.size -= 1;
        true
    }

    fn normalize(mut node: Arc<Node>) -> Option<Arc<Node>> {
        let Node::Branch(branch) = node.as_ref() else {
            return Some(node);
        };
        match (branch.terminal.is_some(), branch.edges.len()) {
            (false, 0) => None,
            (true, 0) => {
                let entry = branch_mut(&mut node).terminal.take().unwrap();
                *Arc::make_mut(&mut node) = Node::Leaf(entry);
                Some(node)
            }
            (false, 1) => {
                let parent = branch_mut(&mut node);
                let mut edge = parent.edges.pop().unwrap();
                if matches!(edge.node.as_ref(), Node::Branch(_)) {
                    let child = branch_mut(&mut edge.node);
                    let mut prefix = std::mem::take(&mut parent.prefix);
                    prefix.push(edge.byte);
                    prefix.append(&mut child.prefix);
                    child.prefix = prefix;
                }
                Some(edge.node)
            }
            _ => Some(node),
        }
    }

    pub fn iter(&self) -> Range<'_> {
        self.range(..)
    }

    /// Bounds are used only during seek. The returned iterator borrows the map,
    /// never the caller's bounds. Invalid bounds follow BTreeMap's panic rules.
    pub fn range<R: RangeBounds<Vec<u8>>>(&self, bounds: R) -> Range<'_> {
        let lower = bounds.start_bound();
        let upper = bounds.end_bound();
        if let (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) =
            (lower, upper)
        {
            assert!(a <= b, "range start is greater than end");
            assert!(
                a != b || !matches!((lower, upper), (Bound::Excluded(_), Bound::Excluded(_))),
                "equal excluded range bounds"
            );
        }
        let mut front = Cursor::seek(self.root.as_deref(), lower, false);
        let mut back = Cursor::seek(self.root.as_deref(), upper, true);
        let first = front.next_entry();
        let last = back.next_entry();
        Range {
            front,
            back,
            first,
            last,
        }
    }

    pub fn seek_le(&self, key: &[u8]) -> Option<(&Vec<u8>, &Vec<u8>)> {
        Cursor::seek_slice(self.root.as_deref(), Some((key, true)), true)
            .next_entry()
            .map(|entry| (&entry.key, &entry.value))
    }
}

enum Task<'a> {
    Node(&'a Node),
    Entry(&'a Entry),
}

struct Cursor<'a> {
    pending: Vec<Task<'a>>,
    reverse: bool,
}

impl<'a> Cursor<'a> {
    fn seek(root: Option<&'a Node>, bound: Bound<&Vec<u8>>, reverse: bool) -> Self {
        let bound = match bound {
            Bound::Unbounded => None,
            Bound::Included(key) => Some((key.as_slice(), true)),
            Bound::Excluded(key) => Some((key.as_slice(), false)),
        };
        Self::seek_slice(root, bound, reverse)
    }

    fn seek_slice(root: Option<&'a Node>, bound: Option<(&[u8], bool)>, reverse: bool) -> Self {
        let mut cursor = Self {
            pending: Vec::new(),
            reverse,
        };
        let Some(mut node) = root else { return cursor };
        let Some((key, inclusive)) = bound else {
            cursor.pending.push(Task::Node(node));
            return cursor;
        };
        let mut depth = 0;
        loop {
            match node {
                Node::Leaf(entry) => {
                    let ordering = entry.key.as_slice().cmp(key);
                    if (inclusive && ordering.is_eq())
                        || if reverse {
                            ordering.is_lt()
                        } else {
                            ordering.is_gt()
                        }
                    {
                        cursor.pending.push(Task::Entry(entry));
                    }
                    break;
                }
                Node::Branch(branch) => {
                    let suffix = &key[depth..];
                    let common = common_prefix(&branch.prefix, suffix);
                    if common < branch.prefix.len() {
                        // A mismatch or an exhausted target orders the entire
                        // subtree; no descendant needs to be inspected.
                        let greater = suffix
                            .get(common)
                            .is_none_or(|&b| branch.prefix[common] > b);
                        if greater != reverse {
                            cursor.pending.push(Task::Node(node));
                        }
                        break;
                    }
                    depth += common;
                    let Some(&byte) = key.get(depth) else {
                        if !reverse {
                            cursor
                                .pending
                                .extend(branch.edges.iter().rev().map(|e| Task::Node(&e.node)));
                        }
                        if inclusive {
                            if let Some(entry) = &branch.terminal {
                                cursor.pending.push(Task::Entry(entry));
                            }
                        }
                        break;
                    };
                    let found = branch.edges.binary_search_by_key(&byte, |edge| edge.byte);
                    let index = match found {
                        Ok(i) | Err(i) => i,
                    };
                    if reverse {
                        if let Some(entry) = &branch.terminal {
                            cursor.pending.push(Task::Entry(entry));
                        }
                        cursor
                            .pending
                            .extend(branch.edges[..index].iter().map(|e| Task::Node(&e.node)));
                    } else {
                        let after = index + usize::from(found.is_ok());
                        cursor.pending.extend(
                            branch.edges[after..]
                                .iter()
                                .rev()
                                .map(|e| Task::Node(&e.node)),
                        );
                    }
                    let Ok(index) = found else { break };
                    node = &branch.edges[index].node;
                    depth += 1;
                }
            }
        }
        cursor
    }

    fn next_entry(&mut self) -> Option<&'a Entry> {
        while let Some(task) = self.pending.pop() {
            match task {
                Task::Entry(entry) | Task::Node(Node::Leaf(entry)) => return Some(entry),
                Task::Node(Node::Branch(branch)) => {
                    if self.reverse {
                        if let Some(entry) = &branch.terminal {
                            self.pending.push(Task::Entry(entry));
                        }
                        self.pending
                            .extend(branch.edges.iter().map(|e| Task::Node(&e.node)));
                    } else {
                        self.pending
                            .extend(branch.edges.iter().rev().map(|e| Task::Node(&e.node)));
                        if let Some(entry) = &branch.terminal {
                            self.pending.push(Task::Entry(entry));
                        }
                    }
                }
            }
        }
        None
    }
}

/// Two seek cursors meet by full bytewise key comparison. Consuming either end
/// cannot return an entry twice, including mixed next/next_back usage.
pub struct Range<'a> {
    front: Cursor<'a>,
    back: Cursor<'a>,
    first: Option<&'a Entry>,
    last: Option<&'a Entry>,
}

impl<'a> Iterator for Range<'a> {
    type Item = (&'a Vec<u8>, &'a Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        let (first, last) = (self.first?, self.last?);
        if first.key > last.key {
            self.first = None;
            self.last = None;
            return None;
        }
        if first.key == last.key {
            self.first = None;
            self.last = None;
        } else {
            self.first = self.front.next_entry();
        }
        Some((&first.key, &first.value))
    }
}

impl DoubleEndedIterator for Range<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let (first, last) = (self.first?, self.last?);
        if first.key > last.key {
            self.first = None;
            self.last = None;
            return None;
        }
        if first.key == last.key {
            self.first = None;
            self.last = None;
        } else {
            self.last = self.back.next_entry();
        }
        Some((&last.key, &last.value))
    }
}

impl FusedIterator for Range<'_> {}

#[cfg(test)]
mod tests;
