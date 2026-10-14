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

    /// `push_back` one element at a time until the vector is `size` long:
    /// each full vector grows by half again.
    pub(crate) fn push_to(&mut self, size: usize) -> Result<(), ImportError> {
        while self.elements < size {
            let grown = vector_growth(self.elements, self.elements + 1);
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

/// A node of `UniqueSet`'s red-black tree; slot 0 is the head, which
/// also stands for every empty child (`_Isnil`).
#[derive(Clone, Copy)]
struct SetNode<T> {
    value: Option<T>,
    left: usize,
    right: usize,
    parent: usize,
    red: bool,
}

const HEAD: usize = 0;

/// `std::set::insert` as MSVC's STL does it: `_Find_lower_bound`, the
/// duplicate test against that bound alone, then `_Insert_node` with its
/// red-black fixup. With a comparator that is not a strict weak order the
/// elements kept and their order depend on the tree's shape, which this
/// keeps exactly.
pub(crate) struct UniqueSet<T> {
    nodes: Vec<SetNode<T>>,
}

impl<T: Copy> UniqueSet<T> {
    pub(crate) fn new() -> Self {
        let head = SetNode { value: None, left: HEAD, right: HEAD, parent: HEAD, red: false };
        UniqueSet { nodes: vec![head] }
    }

    fn value(&self, node: usize) -> &T {
        self.nodes[node].value.as_ref().unwrap()
    }

    /// `insert`: whether the value went in.
    pub(crate) fn insert<E>(&mut self, value: T, less: &mut impl FnMut(&T, &T) -> Result<bool, E>) -> Result<bool, E> {
        let mut parent = self.nodes[HEAD].parent;
        let mut to_right = true;
        let mut bound = HEAD;
        let mut node = parent;
        while node != HEAD {
            parent = node;
            if less(self.value(node), &value)? {
                to_right = true;
                node = self.nodes[node].right;
            } else {
                to_right = false;
                bound = node;
                node = self.nodes[node].left;
            }
        }
        if bound != HEAD && !less(&value, self.value(bound))? {
            return Ok(false);
        }
        self.insert_node(parent, to_right, value);
        Ok(true)
    }

    /// `find`: the node of the element the lower-bound search ends on, if
    /// the key isn't less than it (MSVC's `_Lower_bound_duplicate`).
    pub(crate) fn find<E>(&self, key: &T, less: &mut impl FnMut(&T, &T) -> Result<bool, E>) -> Result<Option<usize>, E> {
        let mut bound = HEAD;
        let mut node = self.nodes[HEAD].parent;
        while node != HEAD {
            if less(self.value(node), key)? {
                node = self.nodes[node].right;
            } else {
                bound = node;
                node = self.nodes[node].left;
            }
        }
        if bound != HEAD && !less(key, self.value(bound))? {
            return Ok(Some(bound));
        }
        Ok(None)
    }

    /// The element a node holds.
    pub(crate) fn get(&self, node: usize) -> &T {
        self.value(node)
    }

    /// Replaces the element a node holds (a map's value, the key kept).
    pub(crate) fn replace(&mut self, node: usize, value: T) {
        self.nodes[node].value = Some(value);
    }

    /// `_Insert_node`.
    fn insert_node(&mut self, parent: usize, to_right: bool, value: T) {
        let new = self.nodes.len();
        self.nodes.push(SetNode { value: Some(value), left: HEAD, right: HEAD, parent, red: true });
        if parent == HEAD {
            self.nodes[HEAD].left = new;
            self.nodes[HEAD].parent = new;
            self.nodes[HEAD].right = new;
            self.nodes[new].red = false;
            return;
        }
        if to_right {
            self.nodes[parent].right = new;
            if parent == self.nodes[HEAD].right {
                self.nodes[HEAD].right = new;
            }
        } else {
            self.nodes[parent].left = new;
            if parent == self.nodes[HEAD].left {
                self.nodes[HEAD].left = new;
            }
        }
        let mut node = new;
        while self.nodes[self.nodes[node].parent].red {
            let parent = self.nodes[node].parent;
            let grandparent = self.nodes[parent].parent;
            if parent == self.nodes[grandparent].left {
                let uncle = self.nodes[grandparent].right;
                if self.nodes[uncle].red {
                    self.nodes[parent].red = false;
                    self.nodes[uncle].red = false;
                    self.nodes[grandparent].red = true;
                    node = grandparent;
                } else {
                    if node == self.nodes[parent].right {
                        node = parent;
                        self.rotate_left(node);
                    }
                    let parent = self.nodes[node].parent;
                    let grandparent = self.nodes[parent].parent;
                    self.nodes[parent].red = false;
                    self.nodes[grandparent].red = true;
                    self.rotate_right(grandparent);
                }
            } else {
                let uncle = self.nodes[grandparent].left;
                if self.nodes[uncle].red {
                    self.nodes[parent].red = false;
                    self.nodes[uncle].red = false;
                    self.nodes[grandparent].red = true;
                    node = grandparent;
                } else {
                    if node == self.nodes[parent].left {
                        node = parent;
                        self.rotate_right(node);
                    }
                    let parent = self.nodes[node].parent;
                    let grandparent = self.nodes[parent].parent;
                    self.nodes[parent].red = false;
                    self.nodes[grandparent].red = true;
                    self.rotate_left(grandparent);
                }
            }
        }
        let root = self.nodes[HEAD].parent;
        self.nodes[root].red = false;
    }

    /// `_Lrotate`: the right child takes the node's place.
    fn rotate_left(&mut self, node: usize) {
        let pivot = self.nodes[node].right;
        let pivot_left = self.nodes[pivot].left;
        self.nodes[node].right = pivot_left;
        if pivot_left != HEAD {
            self.nodes[pivot_left].parent = node;
        }
        let parent = self.nodes[node].parent;
        self.nodes[pivot].parent = parent;
        if node == self.nodes[HEAD].parent {
            self.nodes[HEAD].parent = pivot;
        } else if node == self.nodes[parent].left {
            self.nodes[parent].left = pivot;
        } else {
            self.nodes[parent].right = pivot;
        }
        self.nodes[pivot].left = node;
        self.nodes[node].parent = pivot;
    }

    /// `_Rrotate`: the left child takes the node's place.
    fn rotate_right(&mut self, node: usize) {
        let pivot = self.nodes[node].left;
        let pivot_right = self.nodes[pivot].right;
        self.nodes[node].left = pivot_right;
        if pivot_right != HEAD {
            self.nodes[pivot_right].parent = node;
        }
        let parent = self.nodes[node].parent;
        self.nodes[pivot].parent = parent;
        if node == self.nodes[HEAD].parent {
            self.nodes[HEAD].parent = pivot;
        } else if node == self.nodes[parent].right {
            self.nodes[parent].right = pivot;
        } else {
            self.nodes[parent].left = pivot;
        }
        self.nodes[pivot].right = node;
        self.nodes[node].parent = pivot;
    }

    /// The values in the set's order.
    pub(crate) fn in_order(&self) -> Vec<T> {
        let mut values = Vec::with_capacity(self.nodes.len() - 1);
        let mut stack = Vec::new();
        let mut node = self.nodes[HEAD].parent;
        loop {
            while node != HEAD {
                stack.push(node);
                node = self.nodes[node].left;
            }
            let Some(top) = stack.pop() else {
                return values;
            };
            values.push(*self.value(top));
            node = self.nodes[top].right;
        }
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

    #[test]
    fn a_set_keeps_unique_values_in_order() {
        let mut set = UniqueSet::new();
        let mut less = |a: &u32, b: &u32| Ok::<bool, ()>(a < b);
        for value in [5, 3, 8, 3, 1, 9, 7, 5] {
            set.insert(value, &mut less).unwrap();
        }
        assert_eq!(set.in_order(), vec![1, 3, 5, 7, 8, 9]);
    }

    /// `strncmp(left, right, strlen(left)) < 0`.
    fn prefix_less(left: &[u8], right: &[u8]) -> bool {
        for (index, &byte) in left.iter().enumerate() {
            let other = right.get(index).copied().unwrap_or(0);
            if byte != other {
                return byte < other;
            }
        }
        false
    }

    #[test]
    fn a_set_takes_prefixes_for_duplicates() {
        // Blender's object names: "A" makes "AB" and "AC" duplicates,
        // though "AB" sorts before "AC".
        let names: [&[u8]; 4] = [b"A", b"AB", b"AC", b"B"];
        let mut set = UniqueSet::new();
        let mut less = |a: &usize, b: &usize| Ok::<bool, ()>(prefix_less(names[*a], names[*b]));
        let inserted: Vec<bool> = (0..4).map(|index| set.insert(index, &mut less).unwrap()).collect();
        assert_eq!(inserted, vec![true, false, false, true]);
        assert_eq!(set.in_order(), vec![0, 3]);
    }
}
