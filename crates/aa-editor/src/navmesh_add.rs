//! New navmesh triangles where an edit reshaped the ground inside a triangle.
//!
//! The bake triangulates all the ground (sea and cliffs too: a block of open sea is 6 triangles): walkability lives in
//! the links (`max_pass_radius` -1 = a boundary edge) and the forbidden areas, and the server's heights come from the
//! plane of the enclosing triangle ([`crate::navmesh`] module docs). [`crate::navmesh::adjust_net`] moves corners that
//! sat on the ground and blocks / reopens links, but a hill raised inside a big triangle, or a valley cut through one,
//! is invisible to that plane: NPCs would float over the valley and sink into the hill, and the triangle's links say
//! nothing about the new slopes. [`refine`] splits such triangles:
//!
//! 1. A triangle **needs splitting** when a point of the 4 m lattice inside it (at least [`CLEARANCE`] from its edges)
//!    has ground that moved more than [`TOLERANCE`] and lies more than [`TOLERANCE`] off the triangle's plane.
//!    The bake repeats a triangle in the file of every block it reaches into (a sea block holds its neighbours' big
//!    triangles too) and the server merges the copies by XY, so the split only depends on the triangle's world
//!    corners (rounded to the millimetre) and the ground: every file that holds it splits it the same way, and the
//!    children merge again. [`crate::navmesh::export`] therefore also refines the ring of blocks around the edit.
//! 2. The **children** are a constrained Delaunay triangulation (`spade`) of the triangle's corners, points every 4 m
//!    along its edges and the lattice points inside it, where the ground changed plus one ring around them, at the
//!    new ground height; the edges' pieces are constraints, so the children cover the triangle exactly. An edge's
//!    points only depend on its two ends, so the triangle on the other side (which then needs splitting too) gets
//!    the same ones and their children meet edge to edge.
//! 3. **Records** like the bake's (`bai.rs` of the server; values read from main_world): the first child keeps the
//!    triangle's record and id, the others are appended with the next ids (`dir` / `up` zero, `pos` = the centroid at
//!    the ground's height like the bake's nodes, the corner indices, the triangle's navType and flags), new corners
//!    are appended to the verts file (`flags` 1).
//!    Two children sharing an edge get a link each way: `edge_center` = the edge's middle, `max_pass_radius` = half its
//!    length (as the bake's) when the ground from one centroid to the other is walkable ([`walkable`]: no hole, no
//!    wall above [`WALL`], nothing steeper than [`MAX_SLOPE`]), else -1; exposure 1, the unread floats `0, 0, 327.66`,
//!    the edge's corner positions in the source triangle, bytes `1, 49`. Children of two split neighbours are linked
//!    the same way where their old link was open (a blocked one stays blocked unless the edit made the ground
//!    walkable). The triangle's other links (both directions) move to the child that owns their edge.
//!
//! The other records keep their bytes and order; the counts grow and the new records are appended.

use std::collections::HashMap;

use anyhow::{Result, bail};
use spade::handles::FixedVertexHandle;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

use crate::navmesh::{BLOCK, Ground, MAX_SLOPE, TOLERANCE, WALL};

/// Lattice step of the points added inside a split triangle (m).
pub const STEP: f32 = 4.0;
/// Added points stay this far from the triangle's edges (m), so no sliver lines them.
pub const CLEARANCE: f32 = 1.0;
/// At most about this many points go into one triangle (the lattice coarsens beyond it).
const MAX_POINTS: usize = 3000;

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn put_i32(b: &mut [u8], o: usize, v: i32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_f32(b: &mut [u8], o: usize, v: f32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// Twice the signed area of `a b c` (XY).
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn inside(p: [f64; 2], t: &[[f64; 2]; 3]) -> bool {
    let (d1, d2, d3) = (cross(t[0], t[1], p), cross(t[1], t[2], p), cross(t[2], t[0], p));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

/// Height of the plane through `t` at `p` (`None` for a degenerate triangle).
fn plane(t: &[[f64; 3]; 3], p: [f64; 2]) -> Option<f64> {
    let d = cross([t[0][0], t[0][1]], [t[1][0], t[1][1]], [t[2][0], t[2][1]]);
    if d.abs() < 1e-9 {
        return None;
    }
    let w1 = cross([t[2][0], t[2][1]], [t[0][0], t[0][1]], p) / d;
    let w2 = cross([t[0][0], t[0][1]], [t[1][0], t[1][1]], p) / d;
    Some(t[0][2] * (1.0 - w1 - w2) + t[1][2] * w1 + t[2][2] * w2)
}

/// Whether the new ground from `a` to `b` (world XY) can be walked: no hole, no step steeper than [`MAX_SLOPE`] between
/// samples a metre apart, and no wall higher than [`WALL`] above the line between the ends' ground.
pub fn walkable(a: [f64; 2], b: [f64; 2], ground: &Ground) -> bool {
    let run = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let n = (run.ceil() as usize).clamp(1, 512);
    let step = run / n as f64;
    let at = |t: f64| ground((a[0] + (b[0] - a[0]) * t) as f32, (a[1] + (b[1] - a[1]) * t) as f32);
    let (Some(za), Some(zb)) = (at(0.0), at(1.0)) else { return false };
    let mut prev: Option<f32> = None;
    for k in 0..=n {
        let t = k as f64 / n as f64;
        let Some((_, z, hole)) = at(t) else { return false };
        if hole || z as f64 - (za.1 as f64 + (zb.1 - za.1) as f64 * t) > WALL as f64 {
            return false;
        }
        if let Some(p) = prev
            && step > 0.0
            && ((z - p).abs() as f64 / step) as f32 > MAX_SLOPE
        {
            return false;
        }
        prev = Some(z);
    }
    true
}

/// What [`refine`] did to one block.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Refined {
    /// Triangles split.
    pub split: usize,
    /// Triangles they became.
    pub children: usize,
    /// Links added between children (both directions counted), and of those, blocked ones.
    pub links: usize,
    pub blocked: usize,
}

/// A parsed `netmission` / `vertsmission` pair: records as bytes, so untouched ones are written back as read.
struct Files {
    header: [u8; 28],
    nodes: Vec<[u8; 60]>,
    links: Vec<[u8; 44]>,
    net_tail: Vec<u8>,
    verts_header: [u8; 4],
    verts: Vec<[u8; 32]>,
    verts_tail: Vec<u8>,
}

impl Files {
    fn parse(net: &[u8], verts: &[u8]) -> Result<Files> {
        if net.len() < 36 || verts.len() < 8 {
            bail!("truncated navmesh");
        }
        let n = i32_at(net, 28).max(0) as usize;
        let lo = 32 + n * 60;
        if lo + 4 > net.len() {
            bail!("truncated netmission");
        }
        let l = i32_at(net, lo).max(0) as usize;
        if lo + 4 + l * 44 > net.len() {
            bail!("truncated netmission links");
        }
        let c = i32_at(verts, 4).max(0) as usize;
        if 8 + c * 32 > verts.len() {
            bail!("truncated vertsmission");
        }
        Ok(Files {
            header: net[..28].try_into()?,
            nodes: (0..n).map(|i| net[32 + i * 60..32 + (i + 1) * 60].try_into().unwrap()).collect(),
            links: (0..l).map(|i| net[lo + 4 + i * 44..lo + 4 + (i + 1) * 44].try_into().unwrap()).collect(),
            net_tail: net[lo + 4 + l * 44..].to_vec(),
            verts_header: verts[..4].try_into()?,
            verts: (0..c).map(|i| verts[8 + i * 32..8 + (i + 1) * 32].try_into().unwrap()).collect(),
            verts_tail: verts[8 + c * 32..].to_vec(),
        })
    }

    fn write(&self) -> (Vec<u8>, Vec<u8>) {
        let mut net = self.header.to_vec();
        net.extend_from_slice(&(self.nodes.len() as i32).to_le_bytes());
        self.nodes.iter().for_each(|r| net.extend_from_slice(r));
        net.extend_from_slice(&(self.links.len() as i32).to_le_bytes());
        self.links.iter().for_each(|r| net.extend_from_slice(r));
        net.extend_from_slice(&self.net_tail);
        let mut verts = self.verts_header.to_vec();
        verts.extend_from_slice(&(self.verts.len() as i32).to_le_bytes());
        self.verts.iter().for_each(|r| verts.extend_from_slice(r));
        verts.extend_from_slice(&self.verts_tail);
        (net, verts)
    }

    fn vert(&self, k: i32) -> Option<[f32; 3]> {
        let r = self.verts.get(usize::try_from(k).ok()?)?;
        Some([f32_at(r, 0), f32_at(r, 4), f32_at(r, 8)])
    }

    fn corners(&self, node: usize) -> [i32; 3] {
        let r = &self.nodes[node];
        [i32_at(r, 44), i32_at(r, 48), i32_at(r, 52)]
    }
}

/// A triangle to split: its node, corner vertex indices, the world corners and points (corners first) and the
/// children as indices into them.
struct Split {
    node: usize,
    corners: [i32; 3],
    points: Vec<[f64; 3]>,
    children: Vec<[usize; 3]>,
}

/// World XY to a tenth of a millimetre: the key the server's merge distance (0.1 mm) amounts to.
fn key(p: [f64; 3]) -> (i64, i64) {
    ((p[0] * 10000.0).round() as i64, (p[1] * 10000.0).round() as i64)
}

/// Splits the triangles of block `block` whose plane no longer follows the (edited) ground. `net` / `verts` are the
/// block's adjusted files and are rewritten when anything was split.
pub fn refine(net: &mut Vec<u8>, verts: &mut Vec<u8>, block: (u32, u32), ground: &Ground) -> Result<Refined> {
    let mut f = Files::parse(net, verts)?;
    let origin = (block.0 as f64 * BLOCK as f64, block.1 as f64 * BLOCK as f64);
    let mm = |v: f64| (v * 1000.0).round() / 1000.0;
    let mut splits = Vec::new();
    for t in 0..f.nodes.len() {
        let corners = f.corners(t);
        let Some(cv) = corners.iter().map(|&k| f.vert(k)).collect::<Option<Vec<_>>>() else { continue };
        // world corners to the millimetre: the same in every file that repeats the triangle
        let w: [[f64; 3]; 3] = [0, 1, 2].map(|i| [mm(cv[i][0] as f64 + origin.0), mm(cv[i][1] as f64 + origin.1), cv[i][2] as f64]);
        if cross([w[0][0], w[0][1]], [w[1][0], w[1][1]], [w[2][0], w[2][1]]).abs() < 1e-6 {
            continue;
        }
        let Some((edges, inner)) = lattice(&w, ground) else { continue };
        let Some(children) = triangulate(&w, &edges, &inner) else { continue };
        let points = w.iter().copied().chain(edges.into_iter().flatten()).chain(inner).collect();
        splits.push(Split { node: t, corners, points, children });
    }
    if splits.is_empty() {
        return Ok(Refined::default());
    }
    let done = apply(&mut f, &splits, ground, origin);
    (*net, *verts) = f.write();
    Ok(done)
}

/// New ground `z` lies off the navmesh height `line`.
fn misfit(z: f64, line: f64) -> bool {
    (z - line).abs() > TOLERANCE as f64
}

/// The points to add to triangle `w` (world, z = new ground): per edge (from corner k to k + 1, listed from the edge's
/// lower end) and inside; `None` when its plane still fits the ground. An edge's points only depend on its two ends, so
/// the triangle on its other side gets the same ones.
#[allow(clippy::type_complexity)]
fn lattice(w: &[[f64; 3]; 3], ground: &Ground) -> Option<([Vec<[f64; 3]>; 3], Vec<[f64; 3]>)> {
    let w2 = w.map(|p| [p[0], p[1]]);
    let changed = |p: [f64; 2]| ground(p[0] as f32, p[1] as f32).map(|(old, new, _)| ((new - old).abs() > TOLERANCE, new as f64));
    let mut any_misfit = false;

    // edges: points every STEP from the lower end, kept where changed or next to a changed one
    let edges = [0, 1, 2].map(|k| {
        let (mut a, mut b) = (w[k], w[(k + 1) % 3]);
        if (b[0], b[1]) < (a[0], a[1]) {
            std::mem::swap(&mut a, &mut b);
        }
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let n = (len / STEP as f64).ceil() as usize;
        let samples: Vec<([f64; 3], bool)> = (1..n)
            .filter_map(|i| {
                let t = i as f64 / n as f64;
                if t * len < CLEARANCE as f64 || (1.0 - t) * len < CLEARANCE as f64 {
                    return None;
                }
                let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                let (c, z) = changed(p)?;
                if c && misfit(z, a[2] + (b[2] - a[2]) * t) {
                    any_misfit = true;
                }
                Some(([p[0], p[1], z], c))
            })
            .collect();
        (0..samples.len())
            .filter(|&i| (i.saturating_sub(1)..=(i + 1).min(samples.len() - 1)).any(|j| samples[j].1))
            .map(|i| samples[i].0)
            .collect::<Vec<_>>()
    });

    // inside: a lattice aligned to world multiples of the step (coarser for huge triangles)
    let (x0, x1) = (w2.iter().map(|p| p[0]).fold(f64::MAX, f64::min), w2.iter().map(|p| p[0]).fold(f64::MIN, f64::max));
    let (y0, y1) = (w2.iter().map(|p| p[1]).fold(f64::MAX, f64::min), w2.iter().map(|p| p[1]).fold(f64::MIN, f64::max));
    let area = cross(w2[0], w2[1], w2[2]).abs() / 2.0;
    let mut step = STEP as f64;
    while area / (step * step) > MAX_POINTS as f64 {
        step *= 2.0;
    }
    let clear = |p: [f64; 2]| inside(p, &w2) && (0..3).all(|k| segment_distance(p, w2[k], w2[(k + 1) % 3]) >= CLEARANCE as f64);
    let mut cells: HashMap<(i64, i64), ([f64; 3], bool)> = HashMap::new();
    for i in (x0 / step).floor() as i64..=(x1 / step).ceil() as i64 {
        for j in (y0 / step).floor() as i64..=(y1 / step).ceil() as i64 {
            let p = [i as f64 * step, j as f64 * step];
            if !clear(p) {
                continue;
            }
            let Some((c, z)) = changed(p) else { continue };
            if c && plane(w, p).is_some_and(|l| misfit(z, l)) {
                any_misfit = true;
            }
            cells.insert((i, j), ([p[0], p[1], z], c));
        }
    }
    if !any_misfit {
        return None;
    }
    // the edited points and one ring around them (so the change stays local to its area)
    let mut inner: Vec<((i64, i64), [f64; 3])> = cells
        .iter()
        .filter(|((i, j), (_, c))| *c || (-1..=1).any(|dx| (-1..=1).any(|dy| cells.get(&(i + dx, j + dy)).is_some_and(|c| c.1))))
        .map(|(k, (p, _))| (*k, *p))
        .collect();
    inner.sort_by_key(|(k, _)| *k);
    Some((edges, inner.into_iter().map(|(_, p)| p).collect()))
}

/// The child triangles of `w` with `edges` (points along each edge, from its lower end) and `inner` points, as indices
/// into `w ++ edges ++ inner`. The edges' pieces are constraints, so the children cover the triangle exactly.
fn triangulate(w: &[[f64; 3]; 3], edges: &[Vec<[f64; 3]>; 3], inner: &[[f64; 3]]) -> Option<Vec<[usize; 3]>> {
    let mut cdt: ConstrainedDelaunayTriangulation<Point2<f64>> = ConstrainedDelaunayTriangulation::new();
    let mut index: HashMap<FixedVertexHandle, usize> = HashMap::new();
    let mut handles = Vec::new();
    for (i, p) in w.iter().chain(edges.iter().flatten()).chain(inner).enumerate() {
        let h = cdt.insert(Point2::new(p[0], p[1])).ok()?;
        index.entry(h).or_insert(i);
        handles.push(h);
    }
    let mut at = 3;
    for (k, e) in edges.iter().enumerate() {
        let (a, b) = (handles[k], handles[(k + 1) % 3]);
        let mut chain: Vec<FixedVertexHandle> = handles[at..at + e.len()].to_vec();
        at += e.len();
        // the points run from the edge's lower end
        if (w[k][0], w[k][1]) > (w[(k + 1) % 3][0], w[(k + 1) % 3][1]) {
            chain.reverse();
        }
        let path: Vec<FixedVertexHandle> = std::iter::once(a).chain(chain).chain(std::iter::once(b)).collect();
        for pair in path.windows(2) {
            if pair[0] != pair[1] && cdt.can_add_constraint(pair[0], pair[1]) {
                cdt.add_constraint(pair[0], pair[1]);
            }
        }
    }
    let faces: Vec<[usize; 3]> = cdt.inner_faces().map(|f| f.vertices().map(|v| index[&v.fix()])).collect();
    (faces.len() > 1).then_some(faces)
}

/// A child triangle: its parent's node, id, vertex indices and world corners.
struct Kid {
    parent: usize,
    id: i32,
    verts: [i32; 3],
    world: [[f64; 3]; 3],
}

impl Kid {
    fn centre(&self) -> [f64; 2] {
        [0, 1].map(|k| (self.world[0][k] + self.world[1][k] + self.world[2][k]) / 3.0)
    }
}

/// Replaces the split triangles by their children: records, corners and links.
fn apply(f: &mut Files, splits: &[Split], ground: &Ground, origin: (f64, f64)) -> Refined {
    let mut done = Refined { split: splits.len(), ..Refined::default() };
    let mut next_id = f.nodes.iter().map(|r| i32_at(r, 0)).max().unwrap_or(0) + 1;
    // new corners shared by neighbouring splits (edge points) get one vertex record
    let mut new_vert: HashMap<(i64, i64), i32> = HashMap::new();
    let parent_of: HashMap<i32, usize> = splits.iter().map(|s| (i32_at(&f.nodes[s.node], 0), s.node)).collect();
    let mut kids: Vec<Kid> = Vec::new();
    for s in splits {
        let id = i32_at(&f.nodes[s.node], 0);
        let template = f.nodes[s.node];
        for (n, c) in s.children.iter().enumerate() {
            let verts = c.map(|i| {
                if i < 3 {
                    return s.corners[i];
                }
                let p = s.points[i];
                *new_vert.entry(key(p)).or_insert_with(|| {
                    let mut r = [0u8; 32];
                    put_f32(&mut r, 0, (p[0] - origin.0) as f32);
                    put_f32(&mut r, 4, (p[1] - origin.1) as f32);
                    put_f32(&mut r, 8, p[2] as f32);
                    r[28] = 1;
                    f.verts.push(r);
                    f.verts.len() as i32 - 1
                })
            });
            let world = c.map(|i| s.points[i]);
            let (node, kid) = if n == 0 {
                (s.node, id)
            } else {
                f.nodes.push(template);
                next_id += 1;
                (f.nodes.len() - 1, next_id - 1)
            };
            let r = &mut f.nodes[node];
            put_i32(r, 0, kid);
            let ctr = [0, 1, 2].map(|k| (world[0][k] + world[1][k] + world[2][k]) / 3.0);
            let z = ground(ctr[0] as f32, ctr[1] as f32).map_or(ctr[2], |g| g.1 as f64);
            for (k, v) in [ctr[0] - origin.0, ctr[1] - origin.1, z].into_iter().enumerate() {
                put_f32(r, 28 + k * 4, v as f32);
                put_i32(r, 44 + k * 4, verts[k]);
            }
            put_i32(r, 40, 0);
            kids.push(Kid { parent: s.node, id: kid, verts, world });
        }
    }
    done.children = kids.len();

    // world edge -> the children on it
    let mut by_edge: HashMap<((i64, i64), (i64, i64)), Vec<usize>> = HashMap::new();
    for (n, k) in kids.iter().enumerate() {
        for e in 0..3 {
            let (a, b) = (key(k.world[e]), key(k.world[(e + 1) % 3]));
            by_edge.entry((a.min(b), a.max(b))).or_default().push(n);
        }
    }
    let ids: HashMap<i32, usize> = f.nodes.iter().enumerate().map(|(i, r)| (i32_at(r, 0), i)).collect();
    let mm = |v: f64| (v * 1000.0).round() / 1000.0;
    let world_of = |f: &Files, v: i32| f.vert(v).map(|p| [mm(p[0] as f64 + origin.0), mm(p[1] as f64 + origin.1), p[2] as f64]);

    // the old links of split triangles: to the child owning the whole edge, else (the edge got points: both sides were
    // split) dropped for the children's links below, which take over their open / blocked state
    let mut parent_link: HashMap<(usize, usize), f32> = HashMap::new();
    let mut keep = vec![true; f.links.len()];
    for l in 0..f.links.len() {
        let (src, dst) = (i32_at(&f.links[l], 0), i32_at(&f.links[l], 4));
        let (ps, pd) = (parent_of.get(&src).copied(), parent_of.get(&dst).copied());
        if ps.is_none() && pd.is_none() {
            continue;
        }
        if let (Some(a), Some(b)) = (ps, pd) {
            parent_link.insert((a, b), f32_at(&f.links[l], 20));
            keep[l] = false;
            continue;
        }
        // the edge (world) as the source names it
        let corners = match ps {
            Some(p) => splits.iter().find(|x| x.node == p).map(|x| x.corners).unwrap_or_default(),
            None => match ids.get(&src) {
                Some(&s) => f.corners(s),
                None => continue,
            },
        };
        let (ea, eb) = (corners[f.links[l][40] as usize % 3], corners[f.links[l][41] as usize % 3]);
        let (Some(wa), Some(wb)) = (world_of(f, ea), world_of(f, eb)) else { continue };
        let ek = (key(wa).min(key(wb)), key(wa).max(key(wb)));
        let parent = ps.or(pd).unwrap_or_default();
        let Some(&n) = by_edge.get(&ek).and_then(|v| v.iter().find(|&&n| kids[n].parent == parent)) else {
            // the edge got points but the neighbour was not split: nothing meets it edge to edge any more
            keep[l] = false;
            continue;
        };
        let k = &kids[n];
        let r = &mut f.links[l];
        if ps.is_some() {
            put_i32(r, 0, k.id);
            let at = |w: [f64; 3]| k.world.iter().position(|&p| key(p) == key(w)).unwrap_or(0) as u8;
            r[40] = at(wa);
            r[41] = at(wb);
        } else {
            put_i32(r, 4, k.id);
        }
    }
    let mut i = 0;
    f.links.retain(|_| {
        i += 1;
        keep[i - 1]
    });

    // links between children sharing an edge: open when the ground between their centres can be walked (between two
    // split triangles also only when their old link was open, or the edit made the ground walkable)
    let mut edges: Vec<_> = by_edge.into_iter().filter(|(_, v)| v.len() == 2).collect();
    edges.sort();
    for (_, pair) in edges {
        let (p, q) = (&kids[pair[0]], &kids[pair[1]]);
        let open_now = walkable(p.centre(), q.centre(), ground);
        let open = if p.parent == q.parent {
            open_now
        } else {
            match parent_link.get(&(p.parent, q.parent)) {
                None => continue,
                Some(&r) if r >= 0.0 => open_now,
                Some(_) => open_now && !walkable_old(p.centre(), q.centre(), ground),
            }
        };
        let shared: Vec<usize> = (0..3).filter(|&i| q.world.iter().any(|&w| key(w) == key(p.world[i]))).collect();
        if shared.len() != 2 {
            continue;
        }
        let (pa, pb) = (p.world[shared[0]], p.world[shared[1]]);
        let (va, vb) = (p.verts[shared[0]], p.verts[shared[1]]);
        let len = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt() as f32;
        let radius = if open { (len / 2.0 * 100.0).round() / 100.0 } else { -1.0 };
        let mid = [((pa[0] + pb[0]) / 2.0 - origin.0) as f32, ((pa[1] + pb[1]) / 2.0 - origin.1) as f32, ((pa[2] + pb[2]) / 2.0) as f32];
        for (s, d) in [(p, q), (q, p)] {
            let mut r = [0u8; 44];
            put_i32(&mut r, 0, s.id);
            put_i32(&mut r, 4, d.id);
            for (k, v) in mid.into_iter().chain([radius, 1.0, 0.0, 0.0, 327.66]).enumerate() {
                put_f32(&mut r, 8 + k * 4, v);
            }
            let at = |v: i32, w: [f64; 3]| s.verts.iter().position(|&x| x == v).or_else(|| s.world.iter().position(|&p| key(p) == key(w))).unwrap_or(0) as u8;
            r[40] = at(va, pa);
            r[41] = at(vb, pb);
            r[42] = 1;
            r[43] = 49;
            f.links.push(r);
            done.links += 1;
            done.blocked += (!open) as usize;
        }
    }
    done
}

/// [`walkable`] on the ground before the edit.
fn walkable_old(a: [f64; 2], b: [f64; 2], ground: &Ground) -> bool {
    walkable(a, b, &|x, y| ground(x, y).map(|(old, _, _)| (old, old, false)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Parsed file: nodes `(id, pos, corners)`, links `(src, dst, radius, edge)`, verts.
    type Parsed = (Vec<(i32, [f32; 3], [i32; 3])>, Vec<(i32, i32, f32, (u8, u8))>, Vec<[f32; 3]>);

    fn parse(net: &[u8], verts: &[u8]) -> Parsed {
        let f = Files::parse(net, verts).unwrap();
        let nodes = f.nodes.iter().map(|r| (i32_at(r, 0), [f32_at(r, 28), f32_at(r, 32), f32_at(r, 36)], [i32_at(r, 44), i32_at(r, 48), i32_at(r, 52)])).collect();
        let links = f.links.iter().map(|r| (i32_at(r, 0), i32_at(r, 4), f32_at(r, 20), (r[40], r[41]))).collect();
        let vs = (0..f.verts.len() as i32).map(|k| f.vert(k).unwrap()).collect();
        (nodes, links, vs)
    }

    /// Unique ids; every link joins two nodes of the file, has its reverse, and names an edge (by vertex or XY) both
    /// triangles have; every node lies inside its triangle and no two triangles overlap at a node. With `old` = (last
    /// old id, old vertex count) only links and nodes touching new records are checked (the bake has a few odd ones).
    fn check(p: &Parsed, old: Option<(i32, usize)>) {
        let (nodes, links, vs) = p;
        let fresh = |id: i32| {
            old.is_none_or(|(max, nv)| id > max || nodes.iter().any(|n| n.0 == id && n.2.iter().any(|&k| k as usize >= nv)))
        };
        let by_id: HashMap<i32, &(i32, [f32; 3], [i32; 3])> = nodes.iter().map(|n| (n.0, n)).collect();
        assert_eq!(by_id.len(), nodes.len(), "unique ids");
        let pairs: HashSet<(i32, i32)> = links.iter().map(|l| (l.0, l.1)).collect();
        let xy = |k: i32| [vs[k as usize][0], vs[k as usize][1]];
        for &(s, d, _, (a, b)) in links {
            let (src, dst) = (by_id[&s], by_id[&d]);
            if !fresh(s) && !fresh(d) {
                continue;
            }
            assert!(pairs.contains(&(d, s)), "reverse of {s}->{d}");
            for v in [src.2[a as usize], src.2[b as usize]] {
                assert!(dst.2.iter().any(|&k| xy(k) == xy(v)), "link {s}->{d}: edge corner {v} not in the target");
            }
        }
        for (id, pos, c) in nodes.iter().filter(|n| fresh(n.0)) {
            let t = c.map(|k| [xy(k)[0] as f64, xy(k)[1] as f64]);
            let p = [pos[0] as f64, pos[1] as f64];
            assert!(inside(p, &t), "node {id} inside its triangle");
            let others = nodes.iter().filter(|n| n.0 != *id && inside(p, &n.2.map(|k| [xy(k)[0] as f64, xy(k)[1] as f64]))).count();
            assert_eq!(others, 0, "node {id} inside another triangle");
        }
    }

    /// The files of `tris` (corners as vertex indices into `verts`) with a link each way across every shared edge.
    fn files(verts: &[[f32; 3]], tris: &[[i32; 3]], radius: f32) -> (Vec<u8>, Vec<u8>) {
        let mut net = 55i32.to_le_bytes().to_vec();
        for v in [0.0f32, 0.0, 0.0, 8192.0, 8192.0, 0.0] {
            net.extend_from_slice(&v.to_le_bytes());
        }
        net.extend_from_slice(&(tris.len() as i32).to_le_bytes());
        for (i, t) in tris.iter().enumerate() {
            let mut r = [0u8; 60];
            put_i32(&mut r, 0, i as i32 + 1);
            for k in 0..3 {
                put_f32(&mut r, 28 + k * 4, t.iter().map(|&c| verts[c as usize][k]).sum::<f32>() / 3.0);
                put_i32(&mut r, 44 + k * 4, t[k]);
            }
            r[56] = 2;
            net.extend_from_slice(&r);
        }
        let mut links = Vec::new();
        for (i, a) in tris.iter().enumerate() {
            for (j, b) in tris.iter().enumerate() {
                let shared: Vec<u8> = (0..3u8).filter(|&k| b.contains(&a[k as usize])).collect();
                if i != j && shared.len() == 2 {
                    let mut r = [0u8; 44];
                    put_i32(&mut r, 0, i as i32 + 1);
                    put_i32(&mut r, 4, j as i32 + 1);
                    put_f32(&mut r, 20, radius);
                    r[40] = shared[0];
                    r[41] = shared[1];
                    links.push(r);
                }
            }
        }
        net.extend_from_slice(&(links.len() as i32).to_le_bytes());
        links.iter().for_each(|r| net.extend_from_slice(r));
        let mut v = 2i32.to_le_bytes().to_vec();
        v.extend_from_slice(&(verts.len() as i32).to_le_bytes());
        for p in verts {
            let mut r = [0u8; 32];
            (0..3).for_each(|k| put_f32(&mut r, k * 4, p[k]));
            r[28] = 1;
            v.extend_from_slice(&r);
        }
        (net, v)
    }

    /// Two flat triangles over [0, 200]^2.
    fn square() -> (Vec<u8>, Vec<u8>) {
        files(&[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0], [200.0, 200.0, 0.0], [0.0, 200.0, 0.0]], &[[0, 1, 2], [0, 2, 3]], 70.71)
    }

    /// The height of the triangle at `p` (the server's `enclosing`).
    fn height(p: &Parsed, x: f32, y: f32) -> Option<f32> {
        let (nodes, _, vs) = p;
        nodes.iter().find_map(|n| {
            let t = n.2.map(|k| [vs[k as usize][0] as f64, vs[k as usize][1] as f64, vs[k as usize][2] as f64]);
            inside([x as f64, y as f64], &t.map(|q| [q[0], q[1]])).then(|| plane(&t, [x as f64, y as f64])).flatten().map(|z| z as f32)
        })
    }

    fn hill(x: f32, y: f32) -> Option<(f32, f32, bool)> {
        let d = ((x - 140.0).powi(2) + (y - 60.0).powi(2)).sqrt();
        ((0.0..=256.0).contains(&x) && (0.0..=256.0).contains(&y)).then(|| (0.0, (10.0 - d / 3.0).max(0.0), false))
    }

    #[test]
    fn a_hill_inside_a_triangle_splits_it() {
        let (mut net, mut verts) = square();
        let before = parse(&net, &verts);
        check(&before, None);
        let r = refine(&mut net, &mut verts, (0, 0), &hill).unwrap();
        assert_eq!(r.split, 1, "only the triangle under the hill");
        assert!(r.children > 20, "{r:?}");
        let after = parse(&net, &verts);
        check(&after, None);
        // the server's height now follows the hill
        for (x, y) in [(140.0, 60.0), (145.0, 55.0), (132.0, 66.0), (150.0, 70.0)] {
            let (_, g, _) = hill(x, y).unwrap();
            let h = height(&after, x, y).unwrap();
            assert!((h - g).abs() < TOLERANCE, "{x},{y}: navmesh {h}, ground {g}");
        }
        // the other triangle and its link survive, now to the child on the diagonal
        assert_eq!(after.0[1], before.0[1]);
        let to = after.1.iter().find(|l| l.0 == 2).unwrap();
        assert!(after.0.iter().find(|n| n.0 == to.1).unwrap().2.contains(&0));
        // the hill is gentle: all links open
        assert_eq!(r.blocked, 0);
        // a walk from triangle 2 reaches every child
        let mut seen = HashSet::from([2]);
        let mut todo = vec![2];
        while let Some(n) = todo.pop() {
            for l in after.1.iter().filter(|l| l.0 == n && l.2 >= 0.0) {
                if seen.insert(l.1) {
                    todo.push(l.1);
                }
            }
        }
        assert_eq!(seen.len(), after.0.len());
        // nothing changes a second time
        let (n2, v2) = (net.clone(), verts.clone());
        let again = refine(&mut net, &mut verts, (0, 0), &hill).unwrap();
        assert_eq!(again.split, 0);
        assert_eq!((net, verts), (n2, v2));
    }

    #[test]
    fn a_hill_on_a_shared_edge_splits_both_sides() {
        let (mut net, mut verts) = square();
        // centred on the diagonal both triangles share
        let ridge = |x: f32, y: f32| {
            let d = ((x - 100.0).powi(2) + (y - 100.0).powi(2)).sqrt();
            ((0.0..=256.0).contains(&x) && (0.0..=256.0).contains(&y)).then(|| (0.0, (8.0 - d / 4.0).max(0.0), false))
        };
        let r = refine(&mut net, &mut verts, (0, 0), &ridge).unwrap();
        assert_eq!(r.split, 2, "{r:?}");
        let p = parse(&net, &verts);
        check(&p, None);
        for (x, y) in [(100.0, 100.0), (104.0, 104.0), (96.0, 98.0)] {
            let (_, g, _) = ridge(x, y).unwrap();
            let h = height(&p, x, y).unwrap();
            assert!((h - g).abs() < TOLERANCE, "{x},{y}: navmesh {h}, ground {g}");
        }
        // the two halves are joined across the diagonal: one walk reaches every node
        let mut seen = HashSet::from([p.0[0].0]);
        let mut todo = vec![p.0[0].0];
        while let Some(n) = todo.pop() {
            for l in p.1.iter().filter(|l| l.0 == n && l.2 >= 0.0) {
                if seen.insert(l.1) {
                    todo.push(l.1);
                }
            }
        }
        assert_eq!(seen.len(), p.0.len());
    }

    #[test]
    fn a_cliff_inside_a_triangle_blocks_the_links_over_it() {
        let (mut net, mut verts) = square();
        let cliff = |x: f32, y: f32| ((0.0..=256.0).contains(&x) && (0.0..=256.0).contains(&y)).then_some((0.0, if x > 120.0 && y < 80.0 { 12.0 } else { 0.0 }, false));
        let r = refine(&mut net, &mut verts, (0, 0), &cliff).unwrap();
        assert_eq!(r.split, 1);
        assert!(r.blocked > 0 && r.blocked < r.links, "{r:?}");
        check(&parse(&net, &verts), None);
    }

    #[test]
    fn fitting_triangles_stay() {
        // ground raised 5 m under triangles whose corners moved with it: the plane fits
        let (mut net, mut verts) = files(&[[0.0, 0.0, 5.0], [200.0, 0.0, 5.0], [200.0, 200.0, 5.0], [0.0, 200.0, 5.0]], &[[0, 1, 2], [0, 2, 3]], 70.71);
        let (n0, v0) = (net.clone(), verts.clone());
        let up = |_: f32, _: f32| Some((0.0, 5.0, false));
        assert_eq!(refine(&mut net, &mut verts, (0, 0), &up).unwrap().split, 0);
        assert_eq!((net, verts), (n0, v0));
    }

    #[test]
    fn every_copy_of_a_triangle_splits_the_same() {
        // the same world triangle in the files of blocks (0, 0) and (1, 0): local corners differ by the block origin
        let world = [[100.0f32, 20.0, 0.0], [300.0, 20.0, 0.0], [200.0, 180.0, 0.0]];
        let local = |dx: f32| world.map(|p| [p[0] - dx, p[1], p[2]]);
        let bump = |x: f32, y: f32| Some((0.0, (8.0 - ((x - 210.0).powi(2) + (y - 70.0).powi(2)).sqrt() / 4.0).max(0.0), false));
        let mut centres = Vec::new();
        for (b, dx) in [((0u32, 0u32), 0.0f32), ((1, 0), 256.0)] {
            let (mut net, mut verts) = files(&local(dx), &[[0, 1, 2]], 0.0);
            let r = refine(&mut net, &mut verts, b, &bump).unwrap();
            assert_eq!(r.split, 1);
            let p = parse(&net, &verts);
            check(&p, None);
            let mut c: Vec<(i64, i64)> = p.0.iter().map(|n| (((n.1[0] + dx) as f64 * 1000.0).round() as i64, (n.1[1] as f64 * 1000.0).round() as i64)).collect();
            c.sort();
            centres.push(c);
        }
        assert_eq!(centres[0].len(), centres[1].len());
        // the server merges nodes closer than 0.1 mm: every child has its twin
        for (a, b) in centres[0].iter().zip(&centres[1]) {
            assert!((a.0 - b.0).abs() <= 1 && (a.1 - b.1).abs() <= 1, "{a:?} {b:?}");
        }
    }

    #[test]
    fn real_sea_block_gets_an_island() {
        if !std::path::Path::new(aa_pak::DEFAULT_PAK).exists() && std::env::var_os("AA_PAK").is_none() {
            return;
        }
        // block 062_057: open sea east of the Nuia start, 6 triangles
        let dir = crate::navmesh::block_dir("main_world", (62, 57));
        let mut net = crate::navmesh::original(&format!("{dir}/netmission0.bai")).unwrap();
        let mut verts = crate::navmesh::original(&format!("{dir}/vertsmission0.bai")).unwrap();
        let (n0, v0) = (net.clone(), verts.clone());
        let q = aa_world::TerrainQuery::new("main_world");
        let (cx, cy) = (62.0 * 256.0 + 128.0, 57.0 * 256.0 + 128.0);
        let island = |x: f32, y: f32| {
            let h = q.height_at(x, y)?;
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            Some((h, if d < 60.0 { h.max(104.0 + (60.0 - d) / 6.0) } else { h }, false))
        };
        let r = refine(&mut net, &mut verts, (62, 57), &island).unwrap();
        println!("062_057: {r:?}");
        assert!(r.split > 0, "{r:?}");
        let p = parse(&net, &verts);
        let old = parse(&n0, &v0);
        check(&p, Some((old.0.iter().map(|n| n.0).max().unwrap(), old.2.len())));
        let h = height(&p, cx - 62.0 * 256.0, cy - 57.0 * 256.0).unwrap();
        assert!((h - 114.0).abs() < TOLERANCE, "island top {h}");
        // the original vertices are unchanged, the split triangles became their children
        assert!(p.2[..old.2.len()] == old.2[..]);
        assert_eq!(p.0.len(), old.0.len() + r.children - r.split);
    }
}
