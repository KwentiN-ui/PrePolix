//! A uniform grid over the bounding boxes of items, so the item nearest to a point is
//! found among a few neighbouring cells instead of among all items.

pub(super) struct Grid {
    origin: [f64; 3],
    cell: f64,
    dims: [usize; 3],
    /// Items overlapping each cell, x fastest.
    cells: Vec<Vec<usize>>,
}

impl Grid {
    /// A grid over items with the bounding boxes `boxes`, given as lowest and highest
    /// corner; an item is found by its index in `boxes`.
    pub fn new(boxes: &[([f64; 3], [f64; 3])]) -> Self {
        let (mut low, mut high) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        let mut size = 0.0;
        for (l, h) in boxes {
            for k in 0..3 {
                low[k] = low[k].min(l[k]);
                high[k] = high[k].max(h[k]);
            }
            size += (0..3).map(|k| h[k] - l[k]).fold(0.0, f64::max);
        }
        let extent = (0..3).map(|k| high[k] - low[k]).fold(0.0, f64::max);
        if boxes.is_empty() || !extent.is_finite() {
            return Self {
                origin: [0.0; 3],
                cell: 1.0,
                dims: [0; 3],
                cells: Vec::new(),
            };
        }
        // About the size of an item, but not many more cells than items.
        let n = boxes.len() as f64;
        let mut cell = (size / n)
            .max(extent / n.cbrt())
            .max(extent * 1e-9)
            .max(f64::MIN_POSITIVE);
        let dims = loop {
            let dims = [0, 1, 2].map(|k| ((high[k] - low[k]) / cell).floor() + 1.0);
            if dims.iter().product::<f64>() <= 4.0 * n + 64.0 {
                break dims.map(|d| d as usize);
            }
            cell *= 2.0;
        };
        let mut grid = Self {
            origin: low,
            cell,
            dims,
            cells: vec![Vec::new(); dims.iter().product()],
        };
        for (item, (l, h)) in boxes.iter().enumerate() {
            let (from, to) = (grid.coords(*l), grid.coords(*h));
            for z in from[2]..=to[2] {
                for y in from[1]..=to[1] {
                    for x in from[0]..=to[0] {
                        let index = grid.index([x, y, z]);
                        grid.cells[index].push(item);
                    }
                }
            }
        }
        grid
    }

    /// The item with the smallest `distance` to `point`, and that distance; of equally
    /// near items the first. `distance` must not be smaller than the distance from `point`
    /// to the item's bounding box.
    pub fn nearest(
        &self,
        point: [f64; 3],
        mut distance: impl FnMut(usize) -> f64,
    ) -> Option<(usize, f64)> {
        if self.cells.is_empty() {
            return None;
        }
        let centre = self.coords(point);
        let reach = (0..3)
            .map(|k| centre[k].max(self.dims[k] - 1 - centre[k]))
            .max()
            .unwrap_or(0);
        let mut best: Option<(usize, f64)> = None;
        for r in 0..=reach {
            self.shell(centre, r, |cell| {
                for &item in cell {
                    let d = distance(item);
                    let better = best.is_none_or(|(b, bd)| match d.total_cmp(&bd) {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Equal => item < b,
                        std::cmp::Ordering::Greater => false,
                    });
                    if better {
                        best = Some((item, d));
                    }
                }
            });
            // Items not seen yet lie in cells further out, at least r cells away.
            if best.is_some_and(|(_, d)| d < r as f64 * self.cell) {
                break;
            }
        }
        best
    }

    /// Calls `visit` with the cells exactly `r` cells away from `centre` in some direction.
    fn shell(&self, centre: [usize; 3], r: usize, mut visit: impl FnMut(&[usize])) {
        let range = |k: usize| {
            let c = centre[k] as isize;
            let (r, last) = (r as isize, self.dims[k] as isize - 1);
            (c - r).max(0)..=(c + r).min(last)
        };
        let away = |k: usize, v: isize| (v - centre[k] as isize).unsigned_abs() == r;
        for z in range(2) {
            for y in range(1) {
                let edge = away(2, z) || away(1, y);
                for x in range(0) {
                    if edge || away(0, x) {
                        visit(&self.cells[self.index([x as usize, y as usize, z as usize])]);
                    }
                }
            }
        }
    }

    /// The cell holding `point`, or the nearest one to it.
    fn coords(&self, point: [f64; 3]) -> [usize; 3] {
        [0, 1, 2].map(|k| {
            let c = ((point[k] - self.origin[k]) / self.cell).floor();
            if c.is_nan() {
                0
            } else {
                c.clamp(0.0, (self.dims[k] - 1) as f64) as usize
            }
        })
    }

    fn index(&self, [x, y, z]: [usize; 3]) -> usize {
        x + self.dims[0] * (y + self.dims[1] * z)
    }
}

#[cfg(test)]
mod tests {
    use super::Grid;

    fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
        (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt()
    }

    #[test]
    fn nearest_point_is_the_one_found_by_comparing_all() {
        // A scattered cloud with duplicates, queried inside and far outside of it.
        let points: Vec<[f64; 3]> = (0..500)
            .map(|i| {
                let i = (i % 450) as f64;
                [(i * 7.3) % 13.0, (i * 3.1) % 5.0, (i * 1.7) % 0.5]
            })
            .collect();
        let boxes: Vec<_> = points.iter().map(|&p| (p, p)).collect();
        let grid = Grid::new(&boxes);
        for i in 0..200 {
            let i = i as f64;
            let query = [
                (i * 0.37) % 30.0 - 8.0,
                (i * 1.3) % 12.0 - 3.0,
                i * 0.1 - 5.0,
            ];
            let all = (points.iter().map(|&p| distance(p, query)).enumerate())
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let found = grid.nearest(query, |j| distance(points[j], query));
            assert_eq!(found, all, "query {query:?}");
        }
    }

    #[test]
    fn no_items_nothing_found() {
        assert_eq!(Grid::new(&[]).nearest([0.0; 3], |_| 0.0), None);
        let one = Grid::new(&[([1.0; 3], [1.0; 3])]);
        assert_eq!(one.nearest([5.0; 3], |_| 2.0), Some((0, 2.0)));
    }
}
