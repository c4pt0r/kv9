//! Standalone common-prefix packed-index experiment. Not a production engine.
#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::sync::Arc;

const MAX: usize = 32;
const MIN: usize = MAX / 2;

fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn query_skip(prefix: usize, reference: &[u8], query: &[u8]) -> usize {
    if query.starts_with(&reference[..prefix]) {
        prefix
    } else {
        0
    }
}

fn suffix_cmp(a: &[u8], b: &[u8], skip: usize) -> Ordering {
    a[skip..].cmp(&b[skip..])
}

#[derive(Debug)]
struct Entry {
    key: Vec<u8>,
    value: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Child {
    minimum: Vec<u8>,
    node: Arc<Node>,
}

impl Child {
    fn new(node: Arc<Node>) -> Self {
        Self {
            minimum: node.first_key().to_vec(),
            node,
        }
    }
    fn refresh(&mut self) {
        // Deliberately unchanged: minimum-refresh suppression is a different experiment.
        if self.minimum.as_slice() != self.node.first_key() {
            self.minimum = self.node.first_key().to_vec();
        }
    }
}

#[derive(Clone, Debug)]
enum Body {
    Leaf(Vec<Arc<Entry>>),
    Branch(Vec<Child>),
}

#[derive(Clone, Debug)]
struct Node {
    // A prefix common to ALL subtree keys, not only branch separator minima.
    prefix: usize,
    body: Body,
}

impl Node {
    fn new(body: Body) -> Self {
        let mut node = Self { prefix: 0, body };
        node.reset_prefix();
        node
    }
    fn first_key(&self) -> &[u8] {
        match &self.body {
            Body::Leaf(entries) => &entries[0].key,
            Body::Branch(children) => &children[0].minimum,
        }
    }
    fn last_key(&self) -> &[u8] {
        match &self.body {
            Body::Leaf(entries) => &entries.last().unwrap().key,
            Body::Branch(children) => children.last().unwrap().node.last_key(),
        }
    }
    fn reset_prefix(&mut self) {
        self.prefix = common_prefix_len(self.first_key(), self.last_key());
    }
    fn search_skip(&self, query: &[u8]) -> usize {
        query_skip(self.prefix, self.first_key(), query)
    }
    fn admit_prefix(&mut self, key: &[u8]) {
        let previous = &self.first_key()[..self.prefix];
        if !key.starts_with(previous) {
            self.prefix = common_prefix_len(previous, key);
        }
    }
    fn occupancy(&self) -> usize {
        match &self.body {
            Body::Leaf(entries) => entries.len(),
            Body::Branch(children) => children.len(),
        }
    }
    fn child_for(children: &[Child], key: &[u8], skip: usize) -> usize {
        children
            .partition_point(|child| suffix_cmp(&child.minimum, key, skip) != Ordering::Greater)
            .saturating_sub(1)
    }
    fn insert(node: &mut Arc<Self>, entry: Arc<Entry>) -> (bool, Option<Arc<Self>>) {
        let current = Arc::make_mut(node);
        current.admit_prefix(&entry.key);
        let skip = current.prefix;
        let inserted = match &mut current.body {
            Body::Leaf(entries) => {
                match entries.binary_search_by(|old| suffix_cmp(&old.key, &entry.key, skip)) {
                    Ok(index) => {
                        entries[index] = entry;
                        false
                    }
                    Err(index) => {
                        entries.insert(index, entry);
                        true
                    }
                }
            }
            Body::Branch(children) => {
                let index = Self::child_for(children, &entry.key, skip);
                let (inserted, right) = Self::insert(&mut children[index].node, entry);
                children[index].refresh();
                if let Some(right) = right {
                    children.insert(index + 1, Child::new(right));
                }
                inserted
            }
        };
        let right = if current.occupancy() > MAX {
            let body = match &mut current.body {
                Body::Leaf(entries) => Body::Leaf(entries.split_off(MIN)),
                Body::Branch(children) => Body::Branch(children.split_off(MIN)),
            };
            current.reset_prefix();
            Some(Arc::new(Self::new(body)))
        } else {
            None
        };
        (inserted, right)
    }
    fn borrow_left(left: &mut Arc<Self>, right: &mut Arc<Self>) {
        let (left, right) = (Arc::make_mut(left), Arc::make_mut(right));
        match (&mut left.body, &mut right.body) {
            (Body::Leaf(left), Body::Leaf(right)) => right.insert(0, left.pop().unwrap()),
            (Body::Branch(left), Body::Branch(right)) => right.insert(0, left.pop().unwrap()),
            _ => unreachable!("siblings have equal depth"),
        }
        left.reset_prefix();
        right.reset_prefix();
    }
    fn borrow_right(left: &mut Arc<Self>, right: &mut Arc<Self>) {
        let (left, right) = (Arc::make_mut(left), Arc::make_mut(right));
        match (&mut left.body, &mut right.body) {
            (Body::Leaf(left), Body::Leaf(right)) => left.push(right.remove(0)),
            (Body::Branch(left), Body::Branch(right)) => left.push(right.remove(0)),
            _ => unreachable!("siblings have equal depth"),
        }
        left.reset_prefix();
        right.reset_prefix();
    }
    fn merge(left: &mut Arc<Self>, right: Arc<Self>) {
        let left = Arc::make_mut(left);
        match (&mut left.body, Arc::unwrap_or_clone(right).body) {
            (Body::Leaf(left), Body::Leaf(mut right)) => left.append(&mut right),
            (Body::Branch(left), Body::Branch(mut right)) => left.append(&mut right),
            _ => unreachable!("siblings have equal depth"),
        }
        left.reset_prefix();
    }
    fn repair(children: &mut Vec<Child>, index: usize) {
        if children[index].node.occupancy() >= MIN {
            return;
        }
        if index > 0 && children[index - 1].node.occupancy() > MIN {
            let (left, right) = children.split_at_mut(index);
            Self::borrow_left(&mut left[index - 1].node, &mut right[0].node);
            left[index - 1].refresh();
            right[0].refresh();
        } else if index + 1 < children.len() && children[index + 1].node.occupancy() > MIN {
            let (left, right) = children.split_at_mut(index + 1);
            Self::borrow_right(&mut left[index].node, &mut right[0].node);
            left[index].refresh();
            right[0].refresh();
        } else {
            let left = if index == 0 { 0 } else { index - 1 };
            let right = children.remove(left + 1);
            Self::merge(&mut children[left].node, right.node);
            children[left].refresh();
        }
    }
    fn remove_present(node: &mut Arc<Self>, key: &[u8]) {
        let current = Arc::make_mut(node);
        let skip = current.search_skip(key);
        match &mut current.body {
            Body::Leaf(entries) => {
                let index = entries
                    .binary_search_by(|entry| suffix_cmp(&entry.key, key, skip))
                    .unwrap();
                entries.remove(index);
            }
            Body::Branch(children) => {
                let index = Self::child_for(children, key, skip);
                Self::remove_present(&mut children[index].node, key);
                children[index].refresh();
                Self::repair(children, index);
            }
        }
        // Removing keys only takes a subset of the subtree; the old prefix stays valid.
        // The empty root leaf is removed by PackedMap before any later observation.
    }
}

/// Persistent B+tree with fixed node bounds, bytewise keys and shared immutable entries.
/// Clone shares only the root; mutations copy the affected path and any repaired sibling.
#[derive(Clone, Debug, Default)]
pub struct PackedMap {
    root: Option<Arc<Node>>,
    size: usize,
}

impl PackedMap {
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
        let mut node = self.root.as_deref()?;
        loop {
            let skip = node.search_skip(key);
            match &node.body {
                Body::Leaf(entries) => {
                    return entries
                        .binary_search_by(|entry| suffix_cmp(&entry.key, key, skip))
                        .ok()
                        .map(|index| &entries[index].value)
                }
                Body::Branch(children) => {
                    node = &children[Node::child_for(children, key, skip)].node
                }
            }
        }
    }

    pub fn insert(&mut self, key: Vec<u8>, value: Vec<u8>) -> bool {
        let entry = Arc::new(Entry { key, value });
        let Some(root) = &mut self.root else {
            self.root = Some(Arc::new(Node::new(Body::Leaf(vec![entry]))));
            self.size = 1;
            return true;
        };
        let (inserted, right) = Node::insert(root, entry);
        if let Some(right) = right {
            let left = self.root.take().unwrap();
            self.root = Some(Arc::new(Node::new(Body::Branch(vec![
                Child::new(left),
                Child::new(right),
            ]))));
        }
        self.size += usize::from(inserted);
        inserted
    }

    pub fn remove(&mut self, key: &[u8]) -> bool {
        // Present deletes make two descents; this first check avoids cloning on misses.
        if self.get(key).is_none() {
            return false;
        }
        Node::remove_present(self.root.as_mut().unwrap(), key);
        self.size -= 1;
        if self.size == 0 {
            self.root = None;
        } else if let Body::Branch(children) = &self.root.as_deref().unwrap().body {
            if children.len() == 1 {
                self.root = Some(children[0].node.clone());
            }
        }
        true
    }

    pub fn seek_le(&self, target: &[u8]) -> Option<(&Vec<u8>, &Vec<u8>)> {
        let mut node = self.root.as_deref()?;
        loop {
            let skip = node.search_skip(target);
            match &node.body {
                Body::Leaf(entries) => {
                    let index = entries
                        .partition_point(|entry| {
                            suffix_cmp(&entry.key, target, skip) != Ordering::Greater
                        })
                        .checked_sub(1)?;
                    return Some((&entries[index].key, &entries[index].value));
                }
                Body::Branch(children) => {
                    node = &children[Node::child_for(children, target, skip)].node
                }
            }
        }
    }

    pub fn iter(&self) -> Range<'_> {
        Range::new(self.root.as_deref(), &[], None)
    }

    /// Half-open bytewise range. Start is consumed at construction; end is borrowed.
    pub fn range<'a>(&'a self, start: &[u8], end: &'a [u8]) -> Range<'a> {
        if start >= end {
            return Range::new(None, start, Some(end));
        }
        Range::new(self.root.as_deref(), start, Some(end))
    }

    /// Exhaustive structural validation for the standalone experiment, outside timing.
    pub fn validate(&self) -> Shape {
        fn walk(node: &Node, root: bool, depth: usize, shape: &mut Shape) -> (Vec<u8>, Vec<u8>) {
            shape.nodes += 1;
            assert!(node.occupancy() <= MAX);
            assert!(root || node.occupancy() >= MIN);
            let (minimum, maximum) = match &node.body {
                Body::Leaf(entries) => {
                    assert!(!entries.is_empty());
                    assert!(entries.windows(2).all(|w| w[0].key < w[1].key));
                    if shape.leaves == 0 {
                        shape.height = depth;
                    }
                    assert_eq!(shape.height, depth);
                    shape.leaves += 1;
                    shape.entries += entries.len();
                    (
                        entries.first().unwrap().key.clone(),
                        entries.last().unwrap().key.clone(),
                    )
                }
                Body::Branch(children) => {
                    assert!(children.len() >= 2);
                    let mut first = None;
                    let mut previous: Option<Vec<u8>> = None;
                    for child in children {
                        let (minimum, maximum) = walk(&child.node, false, depth + 1, shape);
                        assert_eq!(child.minimum, minimum);
                        if let Some(last) = previous {
                            assert!(last < minimum);
                        }
                        if first.is_none() {
                            first = Some(minimum);
                        }
                        previous = Some(maximum);
                    }
                    (first.unwrap(), previous.unwrap())
                }
            };
            assert!(node.prefix <= common_prefix_len(&minimum, &maximum));
            (minimum, maximum)
        }
        let mut shape = Shape::default();
        if let Some(root) = self.root.as_deref() {
            walk(root, true, 1, &mut shape);
        }
        assert_eq!(shape.entries, self.size);
        shape
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Shape {
    pub nodes: usize,
    pub leaves: usize,
    pub height: usize,
    pub entries: usize,
}

pub struct Range<'a> {
    stack: Vec<(&'a [Child], usize)>,
    leaf: &'a [Arc<Entry>],
    offset: usize,
    end: Option<&'a [u8]>,
    finished: bool,
}

impl<'a> Range<'a> {
    fn new(root: Option<&'a Node>, start: &[u8], end: Option<&'a [u8]>) -> Self {
        let mut range = Self {
            stack: Vec::new(),
            leaf: &[],
            offset: 0,
            end,
            finished: false,
        };
        let Some(mut node) = root else {
            range.finished = true;
            return range;
        };
        loop {
            let skip = node.search_skip(start);
            match &node.body {
                Body::Leaf(entries) => {
                    range.leaf = entries;
                    range.offset = entries.partition_point(|entry| {
                        suffix_cmp(&entry.key, start, skip) == Ordering::Less
                    });
                    return range;
                }
                Body::Branch(children) => {
                    let index = Node::child_for(children, start, skip);
                    range.stack.push((children, index + 1));
                    node = &children[index].node;
                }
            }
        }
    }

    fn next_leaf(&mut self) -> bool {
        while let Some((children, next)) = self.stack.last_mut() {
            if *next == children.len() {
                self.stack.pop();
                continue;
            }
            let mut node: &'a Node = &children[*next].node;
            *next += 1;
            loop {
                match &node.body {
                    Body::Leaf(entries) => {
                        self.leaf = entries;
                        self.offset = 0;
                        return true;
                    }
                    Body::Branch(children) => {
                        self.stack.push((children, 1));
                        node = &children[0].node;
                    }
                }
            }
        }
        false
    }
}

impl<'a> Iterator for Range<'a> {
    type Item = (&'a Vec<u8>, &'a Vec<u8>);
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if self.offset == self.leaf.len() && !self.next_leaf() {
            self.finished = true;
            return None;
        }
        let entry = &self.leaf[self.offset];
        if self.end.is_some_and(|end| entry.key.as_slice() >= end) {
            self.finished = true;
            return None;
        }
        self.offset += 1;
        Some((&entry.key, &entry.value))
    }
}

impl std::iter::FusedIterator for Range<'_> {}

#[cfg(test)]
mod tests;
