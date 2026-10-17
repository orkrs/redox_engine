//! CPU instancing: ordering of the frame's objects and merging of neighbours into instance ranges.
//!
//! The model buffer holds one matrix per object and every shader finds its object by
//! `instance_index`, so objects that share a mesh (and material) can be drawn with one
//! `draw_indexed` as soon as they sit next to each other in the list. [`draw_order`] puts them
//! there once per frame, [`Runs`] finds the neighbours in each draw loop.

use std::ops::Range;

use crate::systems::RenderObject;

/// Permutation that groups the objects of a frame for instancing, or `None` when the list is
/// already in that order.
///
/// Visible objects come first, sorted by (mesh, material); culled objects go last so they cannot
/// split a group. The sort is stable, so objects with equal keys keep their relative order.
pub(crate) fn draw_order(objects: &[RenderObject], visible: &[bool]) -> Option<Vec<usize>> {
    if objects.len() < 2 || visible.len() != objects.len() {
        return None;
    }
    let key = |i: usize| {
        let o = &objects[i];
        (!visible[i], o.mesh_index, o.material_index)
    };
    if (1..objects.len()).all(|i| key(i - 1) <= key(i)) {
        return None;
    }
    let mut order: Vec<usize> = (0..objects.len()).collect();
    order.sort_by_key(|&i| key(i));
    Some(order)
}

/// Reorders `items` so that `items[k]` becomes the old `items[order[k]]`.
pub(crate) fn permute<T: Clone>(items: &[T], order: &[usize]) -> Vec<T> {
    order.iter().map(|&i| items[i].clone()).collect()
}

/// Iterator over runs of consecutive included indices whose neighbours belong to the same group.
///
/// Each item is the instance range (`first..end`) of one draw call.
pub(crate) struct Runs<I, S> {
    next: usize,
    len: usize,
    included: I,
    same_group: S,
}

/// Runs over `0..len`: indices accepted by `included`, merged while `same_group(first, i)` holds.
pub(crate) fn runs<I, S>(len: usize, included: I, same_group: S) -> Runs<I, S>
where
    I: Fn(usize) -> bool,
    S: Fn(usize, usize) -> bool,
{
    Runs {
        next: 0,
        len,
        included,
        same_group,
    }
}

impl<I, S> Iterator for Runs<I, S>
where
    I: Fn(usize) -> bool,
    S: Fn(usize, usize) -> bool,
{
    type Item = Range<u32>;

    fn next(&mut self) -> Option<Range<u32>> {
        while self.next < self.len && !(self.included)(self.next) {
            self.next += 1;
        }
        if self.next >= self.len {
            return None;
        }
        let first = self.next;
        self.next += 1;
        while self.next < self.len
            && (self.included)(self.next)
            && (self.same_group)(first, self.next)
        {
            self.next += 1;
        }
        Some(first as u32..self.next as u32)
    }
}

#[cfg(test)]
mod tests {
    use redox_math::{Aabb, Mat4};

    use super::*;

    fn object(mesh: usize, material: usize) -> RenderObject {
        RenderObject {
            model_matrix: Mat4::IDENTITY,
            color: [1.0; 4],
            mesh_index: mesh,
            material_index: material,
            bounds: Aabb::empty(),
        }
    }

    fn keys(objects: &[RenderObject], order: &[usize]) -> Vec<(usize, usize)> {
        order
            .iter()
            .map(|&i| (objects[i].mesh_index, objects[i].material_index))
            .collect()
    }

    #[test]
    fn a_sorted_list_needs_no_permutation() {
        let objects = [object(0, 0), object(0, 1), object(1, 0)];
        assert_eq!(draw_order(&objects, &[true; 3]), None);
        assert_eq!(draw_order(&[], &[]), None);
        assert_eq!(draw_order(&objects[..1], &[true]), None);
    }

    #[test]
    fn objects_are_grouped_by_mesh_then_material() {
        let objects = [
            object(1, 0),
            object(0, 1),
            object(1, 0),
            object(0, 0),
            object(0, 1),
        ];
        let order = draw_order(&objects, &[true; 5]).unwrap();
        assert_eq!(
            keys(&objects, &order),
            vec![(0, 0), (0, 1), (0, 1), (1, 0), (1, 0)]
        );
    }

    #[test]
    fn the_sort_is_stable() {
        let objects = [object(0, 0), object(1, 0), object(0, 0), object(1, 0)];
        let order = draw_order(&objects, &[true; 4]).unwrap();
        assert_eq!(order, vec![0, 2, 1, 3]);
    }

    #[test]
    fn culled_objects_go_last() {
        let objects = [object(0, 0), object(0, 0), object(0, 0), object(0, 0)];
        let order = draw_order(&objects, &[true, false, true, false]).unwrap();
        assert_eq!(order, vec![0, 2, 1, 3]);
    }

    #[test]
    fn a_mismatched_visibility_list_is_ignored() {
        let objects = [object(1, 0), object(0, 0)];
        assert_eq!(draw_order(&objects, &[true]), None);
    }

    #[test]
    fn permute_applies_the_order() {
        assert_eq!(permute(&["a", "b", "c"], &[2, 0, 1]), vec!["c", "a", "b"]);
    }

    fn collect(len: usize, visible: &[bool], group: &[u32]) -> Vec<Range<u32>> {
        runs(len, |i| visible[i], |a, b| group[a] == group[b]).collect()
    }

    #[test]
    fn neighbours_of_one_group_merge() {
        assert_eq!(collect(4, &[true; 4], &[7; 4]), vec![0..4]);
    }

    #[test]
    fn a_group_change_starts_a_new_run() {
        assert_eq!(
            collect(5, &[true; 5], &[1, 1, 2, 2, 1]),
            vec![0..2, 2..4, 4..5]
        );
    }

    #[test]
    fn hidden_objects_break_runs_and_are_skipped() {
        assert_eq!(
            collect(5, &[true, true, false, true, true], &[1; 5]),
            vec![0..2, 3..5]
        );
        assert_eq!(collect(3, &[false; 3], &[1; 3]), Vec::<Range<u32>>::new());
    }

    #[test]
    fn empty_input_has_no_runs() {
        assert!(collect(0, &[], &[]).is_empty());
    }
}
