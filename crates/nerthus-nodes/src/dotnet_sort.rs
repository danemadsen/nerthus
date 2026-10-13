//! .NET's `List<T>.Sort` / `Array.Sort` (`ArraySortHelper<T>.IntrospectiveSort`).
//! The sort is unstable, so elements that compare equal end up in an order
//! only this exact algorithm reproduces.

use std::cmp::Ordering;

const INTROSORT_SIZE_THRESHOLD: usize = 16;

/// Sorts `items` exactly as .NET's introspective sort does.
pub fn introspective_sort<T: Clone>(items: &mut [T], compare: &impl Fn(&T, &T) -> Ordering) {
    if items.len() > 1 {
        let depth_limit = 2 * ((items.len() as u32).ilog2() as i32 + 1);
        intro_sort(items, depth_limit, compare);
    }
}

fn swap_if_greater<T>(items: &mut [T], compare: &impl Fn(&T, &T) -> Ordering, i: usize, j: usize) {
    if compare(&items[i], &items[j]) == Ordering::Greater {
        items.swap(i, j);
    }
}

fn intro_sort<T: Clone>(items: &mut [T], mut depth_limit: i32, compare: &impl Fn(&T, &T) -> Ordering) {
    let mut partition_size = items.len();
    while partition_size > 1 {
        if partition_size <= INTROSORT_SIZE_THRESHOLD {
            if partition_size == 2 {
                swap_if_greater(items, compare, 0, 1);
                return;
            }
            if partition_size == 3 {
                swap_if_greater(items, compare, 0, 1);
                swap_if_greater(items, compare, 0, 2);
                swap_if_greater(items, compare, 1, 2);
                return;
            }
            insertion_sort(&mut items[..partition_size], compare);
            return;
        }
        if depth_limit == 0 {
            heap_sort(&mut items[..partition_size], compare);
            return;
        }
        depth_limit -= 1;
        let pivot = pick_pivot_and_partition(&mut items[..partition_size], compare);
        intro_sort(&mut items[pivot + 1..partition_size], depth_limit, compare);
        partition_size = pivot;
    }
}

fn pick_pivot_and_partition<T: Clone>(items: &mut [T], compare: &impl Fn(&T, &T) -> Ordering) -> usize {
    let high = items.len() - 1;
    let middle = high >> 1;
    swap_if_greater(items, compare, 0, middle);
    swap_if_greater(items, compare, 0, high);
    swap_if_greater(items, compare, middle, high);
    let pivot = items[middle].clone();
    items.swap(middle, high - 1);
    let mut left = 0usize;
    let mut right = high - 1;
    while left < right {
        loop {
            left += 1;
            if compare(&items[left], &pivot) != Ordering::Less {
                break;
            }
        }
        loop {
            right -= 1;
            if compare(&pivot, &items[right]) != Ordering::Less {
                break;
            }
        }
        if left >= right {
            break;
        }
        items.swap(left, right);
    }
    if left != high - 1 {
        items.swap(left, high - 1);
    }
    left
}

fn heap_sort<T: Clone>(items: &mut [T], compare: &impl Fn(&T, &T) -> Ordering) {
    let count = items.len();
    let mut index = count >> 1;
    while index >= 1 {
        down_heap(items, index, count, compare);
        index -= 1;
    }
    let mut size = count;
    while size > 1 {
        items.swap(0, size - 1);
        down_heap(items, 1, size - 1, compare);
        size -= 1;
    }
}

fn down_heap<T: Clone>(items: &mut [T], mut index: usize, count: usize, compare: &impl Fn(&T, &T) -> Ordering) {
    let value = items[index - 1].clone();
    while index <= count >> 1 {
        let mut child = 2 * index;
        if child < count && compare(&items[child - 1], &items[child]) == Ordering::Less {
            child += 1;
        }
        if compare(&value, &items[child - 1]) != Ordering::Less {
            break;
        }
        items[index - 1] = items[child - 1].clone();
        index = child;
    }
    items[index - 1] = value;
}

fn insertion_sort<T: Clone>(items: &mut [T], compare: &impl Fn(&T, &T) -> Ordering) {
    for index in 0..items.len() - 1 {
        let value = items[index + 1].clone();
        let mut position = index as isize;
        while position >= 0 && compare(&value, &items[position as usize]) == Ordering::Less {
            items[position as usize + 1] = items[position as usize].clone();
            position -= 1;
        }
        items[(position + 1) as usize] = value;
    }
}
