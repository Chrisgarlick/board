//! Layered auto-layout for flows (spec section 5.3). Pure and deterministic.
//!
//! 1. Break cycles: a depth-first search from the sources (in input order)
//!    reverses back edges.
//! 2. Layers: longest path from the sources.
//! 3. Order within layers: barycentre of neighbours in the adjacent layer,
//!    two down-and-up sweeps.
//! 4. Place: fixed gaps (60 between nodes in a layer, 90 between layers),
//!    each node centred in its layer's band. TB grows down, LR right; BT and
//!    RL are mirrored.
//!
//! Work is bounded: O(sweeps × edges) for ordering, no crossing-minimisation
//! loops that could run long on untrusted input.

use super::mermaid::Direction;

pub const NODE_GAP: f64 = 60.0;
pub const LAYER_GAP: f64 = 90.0;
const SWEEPS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// Top-left positions, one per input node, with the whole layout's top-left at (0, 0).
pub fn layout(sizes: &[Size], edges: &[(usize, usize)], direction: Direction) -> Vec<(f64, f64)> {
    let n = sizes.len();
    if n == 0 {
        return Vec::new();
    }
    let dag = acyclic(n, edges);
    let layer = layers(n, &dag);
    let order = ordering(n, &dag, &layer);

    // Place along the layer axis ("main") and across it ("cross").
    let horizontal = matches!(direction, Direction::LeftRight | Direction::RightLeft);
    let main = |s: &Size| if horizontal { s.w } else { s.h };
    let cross = |s: &Size| if horizontal { s.h } else { s.w };

    let mut main_pos = vec![0.0; n];
    let mut cross_pos = vec![0.0; n];
    let mut offset = 0.0;
    for row in &order {
        let band = row.iter().map(|&i| main(&sizes[i])).fold(0.0, f64::max);
        let total: f64 = row.iter().map(|&i| cross(&sizes[i])).sum::<f64>()
            + NODE_GAP * (row.len().saturating_sub(1)) as f64;
        // Centre each row on the cross axis around 0; shifted to 0 later.
        let mut at = -total / 2.0;
        for &i in row {
            main_pos[i] = offset + (band - main(&sizes[i])) / 2.0;
            cross_pos[i] = at;
            at += cross(&sizes[i]) + NODE_GAP;
        }
        offset += band + LAYER_GAP;
    }

    let mut out: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            if horizontal {
                (main_pos[i], cross_pos[i])
            } else {
                (cross_pos[i], main_pos[i])
            }
        })
        .collect();

    // Mirror for BT and RL.
    let extent_x = out
        .iter()
        .zip(sizes)
        .map(|(p, s)| p.0 + s.w)
        .fold(f64::MIN, f64::max);
    let extent_y = out
        .iter()
        .zip(sizes)
        .map(|(p, s)| p.1 + s.h)
        .fold(f64::MIN, f64::max);
    match direction {
        Direction::RightLeft => {
            for (p, s) in out.iter_mut().zip(sizes) {
                p.0 = extent_x - p.0 - s.w;
            }
        }
        Direction::BottomTop => {
            for (p, s) in out.iter_mut().zip(sizes) {
                p.1 = extent_y - p.1 - s.h;
            }
        }
        _ => {}
    }

    // Shift so the top-left of everything is (0, 0), on whole units.
    let min_x = out.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let min_y = out.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    out.iter()
        .map(|p| ((p.0 - min_x).round(), (p.1 - min_y).round()))
        .collect()
}

/// Edges with back edges reversed and self-loops dropped.
fn acyclic(n: usize, edges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut has_incoming = vec![false; n];
    for &(a, b) in edges {
        if a != b && a < n && b < n {
            out_edges[a].push(b);
            has_incoming[b] = true;
        }
    }
    // Visit sources first (in input order), then anything left (pure cycles).
    let starts: Vec<usize> = (0..n).filter(|&i| !has_incoming[i]).chain(0..n).collect();
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Active,
        Done,
    }
    let mut mark = vec![Mark::New; n];
    let mut back = std::collections::HashSet::new();
    for s in starts {
        if mark[s] != Mark::New {
            continue;
        }
        // Iterative DFS: (node, next child index).
        let mut stack = vec![(s, 0usize)];
        mark[s] = Mark::Active;
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            if *next < out_edges[node].len() {
                let child = out_edges[node][*next];
                *next += 1;
                match mark[child] {
                    Mark::New => {
                        mark[child] = Mark::Active;
                        stack.push((child, 0));
                    }
                    Mark::Active => {
                        back.insert((node, child));
                    }
                    Mark::Done => {}
                }
            } else {
                mark[node] = Mark::Done;
                stack.pop();
            }
        }
    }
    edges
        .iter()
        .filter(|&&(a, b)| a != b && a < n && b < n)
        .map(|&(a, b)| {
            if back.contains(&(a, b)) {
                (b, a)
            } else {
                (a, b)
            }
        })
        .collect()
}

/// Longest path from the sources, by relaxing edges in topological order.
fn layers(n: usize, dag: &[(usize, usize)]) -> Vec<usize> {
    let mut indegree = vec![0usize; n];
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in dag {
        out_edges[a].push(b);
        indegree[b] += 1;
    }
    let mut layer = vec![0usize; n];
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|&i| indegree[i] == 0).collect();
    while let Some(i) = queue.pop_front() {
        for &j in &out_edges[i] {
            layer[j] = layer[j].max(layer[i] + 1);
            indegree[j] -= 1;
            if indegree[j] == 0 {
                queue.push_back(j);
            }
        }
    }
    layer
}

/// Nodes per layer, ordered to reduce crossings.
fn ordering(n: usize, dag: &[(usize, usize)], layer: &[usize]) -> Vec<Vec<usize>> {
    let depth = layer.iter().copied().max().unwrap_or(0) + 1;
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for i in 0..n {
        rows[layer[i]].push(i);
    }
    let mut up: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in dag {
        down[a].push(b);
        up[b].push(a);
    }

    let mut pos = vec![0.0; n];
    let reindex = |rows: &Vec<Vec<usize>>, pos: &mut Vec<f64>| {
        for row in rows {
            for (k, &i) in row.iter().enumerate() {
                pos[i] = k as f64;
            }
        }
    };
    reindex(&rows, &mut pos);

    let sort_by = |row: &mut Vec<usize>, neighbours: &Vec<Vec<usize>>, pos: &Vec<f64>| {
        let key: Vec<(usize, f64)> = row
            .iter()
            .map(|&i| {
                let ns = &neighbours[i];
                let b = if ns.is_empty() {
                    pos[i]
                } else {
                    ns.iter().map(|&j| pos[j]).sum::<f64>() / ns.len() as f64
                };
                (i, b)
            })
            .collect();
        let mut key = key;
        // Stable: ties keep their current order.
        key.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        *row = key.into_iter().map(|(i, _)| i).collect();
    };

    for _ in 0..SWEEPS {
        for l in 1..depth {
            sort_by(&mut rows[l], &up, &pos);
            reindex(&rows, &mut pos);
        }
        for l in (0..depth.saturating_sub(1)).rev() {
            sort_by(&mut rows[l], &down, &pos);
            reindex(&rows, &mut pos);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(w: f64, h: f64) -> Size {
        Size { w, h }
    }

    fn overlaps(p: &[(f64, f64)], sizes: &[Size]) -> bool {
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                let (a, b) = (p[i], p[j]);
                if a.0 < b.0 + sizes[j].w
                    && b.0 < a.0 + sizes[i].w
                    && a.1 < b.1 + sizes[j].h
                    && b.1 < a.1 + sizes[i].h
                {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn a_chain_goes_down_in_tb_and_across_in_lr() {
        let sizes = [s(160.0, 72.0), s(150.0, 110.0), s(160.0, 72.0)];
        let edges = [(0, 1), (1, 2)];
        let tb = layout(&sizes, &edges, Direction::TopBottom);
        assert!(tb[0].1 < tb[1].1 && tb[1].1 < tb[2].1);
        assert_eq!(
            tb[1].1 - (tb[0].1 + 72.0),
            LAYER_GAP + 0.0,
            "gap between layers"
        );
        let lr = layout(&sizes, &edges, Direction::LeftRight);
        assert!(lr[0].0 < lr[1].0 && lr[1].0 < lr[2].0);
        let bt = layout(&sizes, &edges, Direction::BottomTop);
        assert!(bt[0].1 > bt[1].1 && bt[1].1 > bt[2].1);
        let rl = layout(&sizes, &edges, Direction::RightLeft);
        assert!(rl[0].0 > rl[2].0);
    }

    #[test]
    fn branches_share_a_layer_without_overlap() {
        // 0 → {1, 2} → 3
        let sizes = [s(160.0, 72.0); 4];
        let p = layout(
            &sizes,
            &[(0, 1), (0, 2), (1, 3), (2, 3)],
            Direction::TopBottom,
        );
        assert_eq!(p[1].1, p[2].1, "siblings in one layer");
        assert!(p[3].1 > p[1].1);
        assert!(!overlaps(&p, &sizes));
        assert_eq!(p.iter().map(|q| q.0).fold(f64::MAX, f64::min), 0.0);
    }

    #[test]
    fn cycles_still_lay_out() {
        // A loop back to the start (a retry) mustn't hang or stack nodes.
        let sizes = [s(160.0, 72.0); 3];
        let p = layout(&sizes, &[(0, 1), (1, 2), (2, 0)], Direction::TopBottom);
        assert!(p[0].1 < p[1].1 && p[1].1 < p[2].1);
        let pure = layout(&sizes, &[(0, 1), (1, 0), (2, 2)], Direction::TopBottom);
        assert!(!overlaps(&pure, &sizes));
    }

    #[test]
    fn deterministic_and_bounded() {
        let sizes = vec![s(160.0, 72.0); 300];
        let edges: Vec<(usize, usize)> = (0..299)
            .flat_map(|i| [(i, i + 1), (i, (i * 7) % 300)])
            .collect();
        let a = layout(&sizes, &edges, Direction::TopBottom);
        let b = layout(&sizes, &edges, Direction::TopBottom);
        assert_eq!(a, b);
        assert!(!overlaps(&a, &sizes));
    }

    #[test]
    fn crossings_are_reduced() {
        // 0→3, 1→2: without reordering the edges would cross.
        let sizes = [s(100.0, 50.0); 4];
        let p = layout(&sizes, &[(0, 3), (1, 2)], Direction::TopBottom);
        assert!(p[0].0 < p[1].0);
        assert!(p[3].0 < p[2].0, "children follow their parents' order");
    }
}
