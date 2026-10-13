//! .NET's `PriorityQueue<TElement, float>` (System.Collections.Generic):
//! a quaternary min-heap. Equal priorities come out in the order its sift
//! steps leave them, which the point placement depends on.

use std::cmp::Ordering;

/// `float.CompareTo`: NaN is smaller than every number and equal to NaN.
fn compare(a: f32, b: f32) -> Ordering {
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    if a == b {
        return Ordering::Equal;
    }
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        _ => Ordering::Greater,
    }
}

const ARITY: usize = 4;

pub struct PriorityQueue<T: Copy> {
    nodes: Vec<(T, f32)>,
}

impl<T: Copy> Default for PriorityQueue<T> {
    fn default() -> Self {
        PriorityQueue { nodes: Vec::new() }
    }
}

impl<T: Copy> PriorityQueue<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// `Enqueue`: the new node moves up past parents with greater
    /// priority.
    pub fn enqueue(&mut self, element: T, priority: f32) {
        let node = (element, priority);
        let mut index = self.nodes.len();
        self.nodes.push(node);
        while index > 0 {
            let parent_index = (index - 1) / ARITY;
            let parent = self.nodes[parent_index];
            if compare(node.1, parent.1) == Ordering::Less {
                self.nodes[index] = parent;
                index = parent_index;
            } else {
                break;
            }
        }
        self.nodes[index] = node;
    }

    /// `Dequeue`: the root; the last node then moves down from the root
    /// past its smallest child (the first of equal children).
    pub fn dequeue(&mut self) -> Option<T> {
        if self.nodes.is_empty() {
            return None;
        }
        let root = self.nodes[0].0;
        let last = self.nodes.pop().expect("not empty");
        let size = self.nodes.len();
        if size == 0 {
            return Some(root);
        }
        let mut index = 0;
        loop {
            let first_child = index * ARITY + 1;
            if first_child >= size {
                break;
            }
            let mut min_child = self.nodes[first_child];
            let mut min_index = first_child;
            let upper = (first_child + ARITY).min(size);
            for child in first_child + 1..upper {
                let candidate = self.nodes[child];
                if compare(candidate.1, min_child.1) == Ordering::Less {
                    min_child = candidate;
                    min_index = child;
                }
            }
            if compare(last.1, min_child.1) != Ordering::Greater {
                break;
            }
            self.nodes[index] = min_child;
            index = min_index;
        }
        self.nodes[index] = last;
        Some(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dequeues_in_priority_order() {
        let mut queue = PriorityQueue::new();
        for (element, priority) in [(1, 5.0), (2, 1.0), (3, 3.0), (4, 1.0), (5, 0.5)] {
            queue.enqueue(element, priority);
        }
        let order: Vec<i32> = std::iter::from_fn(|| queue.dequeue()).collect();
        assert_eq!(order[0], 5);
        assert_eq!(order[4], 1);
    }
}
