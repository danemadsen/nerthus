//! The scanline flood fill the CW engine runs for Sea's edge fill and for
//! flattening lakes.
//!
//! Each span pushed on the queue includes the cell past either end, so the
//! rows above and below are searched one cell beyond the span: the fill is
//! 8-connected.

/// A queued span: first and last column (both inclusive) and the row.
#[derive(Clone, Copy)]
struct Span {
    left: i32,
    right: i32,
    row: i32,
}

pub struct SpanFill {
    size: i32,
    visited: Vec<u8>,
    queue: Vec<Span>,
}

impl SpanFill {
    pub fn new(size: usize) -> SpanFill {
        SpanFill {
            size: size as i32,
            visited: vec![0; size * size],
            queue: Vec::new(),
        }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        (y * self.size + x) as usize
    }

    pub fn is_visited(&self, x: i32, y: i32) -> bool {
        self.visited[self.index(x, y)] != 0
    }

    /// Fills the run through (`x`, `y`), queues it and
    /// returns the column to continue the search from.
    fn scan(&mut self, x: i32, y: i32, test: &impl Fn(i32, i32) -> bool, action: &mut impl FnMut(i32, i32)) -> i32 {
        if self.is_visited(x, y) || !test(x, y) {
            return x + 1;
        }
        let mut left = x;
        loop {
            left -= 1;
            if left < 0 || self.is_visited(left, y) || !test(left, y) {
                break;
            }
            let index = self.index(left, y);
            self.visited[index] = 1;
            action(left, y);
        }
        let mut right = x;
        while right < self.size {
            if self.is_visited(right, y) || !test(right, y) {
                break;
            }
            let index = self.index(right, y);
            self.visited[index] = 1;
            action(right, y);
            right += 1;
        }
        let last = right.min(self.size - 1);
        let span = Span {
            left: left.max(0),
            right: last,
            row: y,
        };
        self.queue.push(span);
        last
    }

    /// Fills everything reachable from (`x`, `y`) through
    /// cells passing `test`, calling `action` once per filled cell.
    pub fn fill(&mut self, x: i32, y: i32, test: &impl Fn(i32, i32) -> bool, action: &mut impl FnMut(i32, i32)) {
        self.queue.clear();
        self.scan(x, y, test, action);
        let mut next = 0;
        while next < self.queue.len() {
            let span = self.queue[next];
            next += 1;
            let above = span.row - 1;
            if above >= 0 {
                let mut column = span.left;
                while column <= span.right {
                    if !self.is_visited(column, above) && test(column, above) {
                        column = self.scan(column, above, test, action);
                        continue;
                    }
                    column += 1;
                }
            }
            let below = span.row + 1;
            if below >= self.size || span.right < span.left {
                continue;
            }
            let mut column = span.left;
            loop {
                if !self.is_visited(column, below) && test(column, below) {
                    column = self.scan(column, below, test, action);
                } else {
                    column += 1;
                }
                if column > span.right {
                    break;
                }
            }
        }
    }
}
