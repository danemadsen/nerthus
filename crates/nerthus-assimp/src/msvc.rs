//! MSVC standard library behaviour the Windows Assimp build has and other
//! standard libraries don't: `std::vector`'s growth and `std::sort`'s
//! order of equal elements.

use crate::scene::{ImportError, check_allocation};

/// `vector::_Calculate_growth`: the capacity after `size` grows past
/// `capacity` (half again, or what is needed).
pub(crate) fn vector_growth(capacity: usize, needed: usize) -> usize {
    let geometric = capacity + capacity / 2;
    geometric.max(needed)
}

/// A `std::vector`'s capacity as MSVC manages it, for the sizes of the
/// allocations behind it: `reserve` allocates exactly what it is asked
/// for, `resize` and `push_back` grow by `vector_growth`. An allocation
/// past the limit fails as `bad_alloc`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VectorCapacity {
    elements: usize,
    element_size: u64,
}

impl VectorCapacity {
    pub(crate) fn new(element_size: u64) -> Self {
        Self { elements: 0, element_size }
    }

    /// `reserve(count)`.
    pub(crate) fn reserve(&mut self, count: usize) -> Result<(), ImportError> {
        if count > self.elements {
            check_allocation(count as u64, self.element_size)?;
            self.elements = count;
        }
        Ok(())
    }

    /// `resize(size)`, or a `push_back` making the vector `size` long.
    pub(crate) fn grow(&mut self, size: usize) -> Result<(), ImportError> {
        if size > self.elements {
            let grown = vector_growth(self.elements, size);
            check_allocation(grown as u64, self.element_size)?;
            self.elements = grown;
        }
        Ok(())
    }
}

/// `_ISORT_MAX`: ranges this small are insertion sorted.
const INSERTION_SORT_MAX: usize = 32;

/// `std::sort` as MSVC's STL implements it (introsort: a median-guess
/// three-way partition, insertion sort for small ranges, heap sort when
/// the recursion runs deep), so equal elements end up in the same order.
pub(crate) fn sort<T: Copy>(items: &mut [T], less: impl Fn(&T, &T) -> bool) {
    let length = items.len();
    sort_range(items, 0, length, length as isize, &less);
}

fn sort_range<T: Copy>(items: &mut [T], mut first: usize, mut last: usize, mut ideal: isize, less: &impl Fn(&T, &T) -> bool) {
    loop {
        if last - first <= INSERTION_SORT_MAX {
            insertion_sort(items, first, last, less);
            return;
        }
        if ideal <= 0 {
            make_heap(items, first, last, less);
            sort_heap(items, first, last, less);
            return;
        }
        let (middle_first, middle_last) = partition_by_median_guess(items, first, last, less);
        // Allow 1.5 log2(N) divisions.
        ideal = (ideal >> 1) + (ideal >> 2);
        if middle_first - first < last - middle_last {
            sort_range(items, first, middle_first, ideal, less);
            first = middle_last;
        } else {
            sort_range(items, middle_last, last, ideal, less);
            last = middle_first;
        }
    }
}

/// `_Insertion_sort_unchecked`.
fn insertion_sort<T: Copy>(items: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    if first == last {
        return;
    }
    let mut middle = first;
    loop {
        middle += 1;
        if middle == last {
            return;
        }
        let value = items[middle];
        if less(&value, &items[first]) {
            // A new earliest element: everything before it moves up.
            items.copy_within(first..middle, first + 1);
            items[first] = value;
        } else {
            let mut hole = middle;
            let mut previous = hole - 1;
            while less(&value, &items[previous]) {
                items[hole] = items[previous];
                hole = previous;
                previous -= 1;
            }
            items[hole] = value;
        }
    }
}

/// `_Med3_unchecked`: the median of three moved to the middle.
fn median_of_three<T: Copy>(items: &mut [T], first: usize, middle: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    if less(&items[middle], &items[first]) {
        items.swap(middle, first);
    }
    if less(&items[last], &items[middle]) {
        items.swap(last, middle);
        if less(&items[middle], &items[first]) {
            items.swap(middle, first);
        }
    }
}

/// `_Guess_median_unchecked` (`last` inclusive): Tukey's ninther above 40
/// elements.
fn guess_median<T: Copy>(items: &mut [T], first: usize, middle: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    let count = last - first;
    if 40 < count {
        let step = (count + 1) >> 3;
        let two_steps = step << 1;
        median_of_three(items, first, first + step, first + two_steps, less);
        median_of_three(items, middle - step, middle, middle + step, less);
        median_of_three(items, last - two_steps, last - step, last, less);
        median_of_three(items, first + step, middle, last - step, less);
    } else {
        median_of_three(items, first, middle, last, less);
    }
}

/// `_Partition_by_median_guess_unchecked`: the range of elements equal
/// to the pivot, with the smaller ones before it and the larger after.
fn partition_by_median_guess<T: Copy>(items: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) -> (usize, usize) {
    let middle = first + ((last - first) >> 1);
    guess_median(items, first, middle, last - 1, less);

    let mut pivot_first = middle;
    let mut pivot_last = pivot_first + 1;
    while first < pivot_first && !less(&items[pivot_first - 1], &items[pivot_first]) && !less(&items[pivot_first], &items[pivot_first - 1]) {
        pivot_first -= 1;
    }
    while pivot_last < last && !less(&items[pivot_last], &items[pivot_first]) && !less(&items[pivot_first], &items[pivot_last]) {
        pivot_last += 1;
    }
    let mut greater_first = pivot_last;
    let mut greater_last = pivot_first;
    loop {
        while greater_first < last {
            if less(&items[pivot_first], &items[greater_first]) {
                greater_first += 1;
                continue;
            }
            if less(&items[greater_first], &items[pivot_first]) {
                break;
            }
            if pivot_last != greater_first {
                items.swap(pivot_last, greater_first);
            }
            pivot_last += 1;
            greater_first += 1;
        }
        while first < greater_last {
            let previous = greater_last - 1;
            if less(&items[previous], &items[pivot_first]) {
                greater_last -= 1;
                continue;
            }
            if less(&items[pivot_first], &items[previous]) {
                break;
            }
            pivot_first -= 1;
            if pivot_first != previous {
                items.swap(pivot_first, previous);
            }
            greater_last -= 1;
        }
        if greater_last == first && greater_first == last {
            return (pivot_first, pivot_last);
        }
        if greater_last == first {
            // No room at the bottom: the pivot rotates up.
            if pivot_last != greater_first {
                items.swap(pivot_first, pivot_last);
            }
            pivot_last += 1;
            items.swap(pivot_first, greater_first);
            pivot_first += 1;
            greater_first += 1;
        } else if greater_first == last {
            // No room at the top: the pivot rotates down.
            greater_last -= 1;
            pivot_first -= 1;
            if greater_last != pivot_first {
                items.swap(greater_last, pivot_first);
            }
            pivot_last -= 1;
            items.swap(pivot_first, pivot_last);
        } else {
            greater_last -= 1;
            items.swap(greater_first, greater_last);
            greater_first += 1;
        }
    }
}

/// `_Push_heap_by_index`: the hole percolated up towards `top`.
fn push_heap_by_index<T: Copy>(items: &mut [T], base: usize, mut hole: usize, top: usize, value: T, less: &impl Fn(&T, &T) -> bool) {
    while top < hole {
        let parent = (hole - 1) >> 1;
        if !less(&items[base + parent], &value) {
            break;
        }
        items[base + hole] = items[base + parent];
        hole = parent;
    }
    items[base + hole] = value;
}

/// `_Pop_heap_hole_by_index`: the hole moved down to the larger child to
/// the bottom, then the value pushed back up.
fn pop_heap_hole_by_index<T: Copy>(items: &mut [T], base: usize, mut hole: usize, bottom: usize, value: T, less: &impl Fn(&T, &T) -> bool) {
    let top = hole;
    let mut index = hole;
    let max_non_leaf = (bottom - 1) >> 1;
    while index < max_non_leaf {
        index = 2 * index + 2;
        if less(&items[base + index], &items[base + index - 1]) {
            index -= 1;
        }
        items[base + hole] = items[base + index];
        hole = index;
    }
    if index == max_non_leaf && bottom.is_multiple_of(2) {
        items[base + hole] = items[base + bottom - 1];
        hole = bottom - 1;
    }
    push_heap_by_index(items, base, hole, top, value, less);
}

/// `_Make_heap_unchecked`.
fn make_heap<T: Copy>(items: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    let bottom = last - first;
    let mut hole = bottom >> 1;
    while hole > 0 {
        hole -= 1;
        let value = items[first + hole];
        pop_heap_hole_by_index(items, first, hole, bottom, value, less);
    }
}

/// `_Sort_heap_unchecked`: the top popped to the end, repeatedly.
fn sort_heap<T: Copy>(items: &mut [T], first: usize, mut last: usize, less: &impl Fn(&T, &T) -> bool) {
    while last - first >= 2 {
        last -= 1;
        let value = items[last];
        items[last] = items[first];
        pop_heap_hole_by_index(items, first, 0, last - first, value, less);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_grow_by_half() {
        let mut capacity = 0;
        let mut capacities = Vec::new();
        for size in 0..10 {
            if size == capacity {
                capacity = vector_growth(capacity, size + 1);
                capacities.push(capacity);
            }
        }
        assert_eq!(capacities, vec![1, 2, 3, 4, 6, 9, 13]);
    }

    #[test]
    fn sorts_like_any_sort_by_key() {
        let mut state = 12345u32;
        let mut values: Vec<(u32, usize)> = (0..5000)
            .map(|index| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                ((state >> 16) % 300, index)
            })
            .collect();
        sort(&mut values, |a, b| a.0 < b.0);
        assert!(values.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        let mut indices: Vec<usize> = values.iter().map(|value| value.1).collect();
        indices.sort();
        assert_eq!(indices, (0..5000).collect::<Vec<_>>());
    }

    #[test]
    fn deep_recursion_falls_back_to_heap_sort() {
        let mut values: Vec<u32> = (0..1000).map(|index| (index * 7919) % 1000).collect();
        sort_range(&mut values, 0, 1000, 0, &|a: &u32, b: &u32| a < b);
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
    }
}
