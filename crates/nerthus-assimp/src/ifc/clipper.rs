//! Clipper 6.4.2 (Angus Johnson), the polygon clipper Assimp bundles, as
//! far as the openings use it: closed paths, union, difference and
//! intersection with both polygon kinds filled non-zero, no options.
//! Integer arithmetic wraps as the x64 build's does and its two sorts are
//! MSVC's. A coordinate past Clipper's range throws out of the import; a
//! failure while clipping gives no polygons.

use std::collections::BinaryHeap;

use super::{Failure, Outcome};
use crate::cast::to_i64;
use crate::msvc;

/// `loRange`: past this, slopes compare in 128 bits.
const LOW_RANGE: i64 = 0x3FFF_FFFF;
/// `hiRange`: past this, a coordinate throws.
const HIGH_RANGE: i64 = 0x3FFF_FFFF_FFFF_FFFF;
/// `HORIZONTAL`: the `Dx` of a horizontal edge.
const HORIZONTAL: f64 = -1.0e40;
/// `Unassigned`: an edge not building an output polygon.
const UNASSIGNED: i32 = -1;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) struct IntPoint {
    pub(super) x: i64,
    pub(super) y: i64,
}

impl IntPoint {
    fn new(x: i64, y: i64) -> Self {
        Self { x, y }
    }
}

pub(super) type Path = Vec<IntPoint>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ClipType {
    Intersection,
    Union,
    Difference,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PolyType {
    Subject,
    Clip,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    RightToLeft,
    LeftToRight,
}

/// `TEdge`: an edge of an input polygon. Links index the clipper's edges.
#[derive(Clone, Copy)]
struct Edge {
    bot: IntPoint,
    curr: IntPoint,
    top: IntPoint,
    dx: f64,
    poly_type: PolyType,
    side: Side,
    wind_delta: i32,
    wind_count: i32,
    wind_count_2: i32,
    out_index: i32,
    next: usize,
    prev: usize,
    next_in_lml: Option<usize>,
    next_in_ael: Option<usize>,
    prev_in_ael: Option<usize>,
    next_in_sel: Option<usize>,
    prev_in_sel: Option<usize>,
}

impl Edge {
    /// `InitEdge`: a zeroed edge at `curr`.
    fn new(curr: IntPoint, next: usize, prev: usize) -> Self {
        Self {
            bot: IntPoint::default(),
            curr,
            top: IntPoint::default(),
            dx: 0.0,
            poly_type: PolyType::Subject,
            side: Side::Left,
            wind_delta: 0,
            wind_count: 0,
            wind_count_2: 0,
            out_index: UNASSIGNED,
            next,
            prev,
            next_in_lml: None,
            next_in_ael: None,
            prev_in_ael: None,
            next_in_sel: None,
            prev_in_sel: None,
        }
    }

    fn is_horizontal(&self) -> bool {
        self.dx == HORIZONTAL
    }
}

#[derive(Clone, Copy)]
struct IntersectNode {
    edge_1: usize,
    edge_2: usize,
    point: IntPoint,
}

#[derive(Clone, Copy)]
struct LocalMinimum {
    y: i64,
    left_bound: usize,
    right_bound: usize,
}

/// `OutRec`: an output polygon. `first_left` indexes the records,
/// `points` and `bottom_point` the output points.
#[derive(Clone, Copy)]
struct OutRec {
    index: i32,
    is_hole: bool,
    first_left: Option<usize>,
    points: Option<usize>,
    bottom_point: Option<usize>,
}

/// `OutPt`: a point in an output polygon's ring.
#[derive(Clone, Copy)]
struct OutPt {
    index: i32,
    point: IntPoint,
    next: usize,
    prev: usize,
}

/// `Join`: output points to join along a shared edge (a ghost join has
/// no second point).
#[derive(Clone, Copy)]
struct Join {
    out_pt_1: Option<usize>,
    out_pt_2: Option<usize>,
    offset_point: IntPoint,
}

/// `Orientation`: whether the path's area isn't negative.
pub(super) fn orientation(path: &[IntPoint]) -> bool {
    area(path) >= 0.0
}

/// `Area` of a path.
fn area(path: &[IntPoint]) -> f64 {
    if path.len() < 3 {
        return 0.0;
    }
    let mut total = 0.0;
    let mut j = path.len() - 1;
    for i in 0..path.len() {
        let sum = path[j].x as f64 + path[i].x as f64;
        let difference = path[j].y as f64 - path[i].y as f64;
        total += sum * difference;
        j = i;
    }
    -total * 0.5
}

/// `Round`: half away from zero, truncated as x64 does.
fn round(value: f64) -> i64 {
    if value < 0.0 { to_i64(value - 0.5) } else { to_i64(value + 0.5) }
}

/// `Int128Mul`: the 128-bit product as Clipper forms it (the middle
/// partial products' carry is lost), as (hi, lo).
fn int128_mul(lhs: i64, rhs: i64) -> (i64, u64) {
    let negate = (lhs < 0) != (rhs < 0);
    let lhs = if lhs < 0 { lhs.wrapping_neg() } else { lhs };
    let rhs = if rhs < 0 { rhs.wrapping_neg() } else { rhs };
    let int1_hi = (lhs as u64) >> 32;
    let int1_lo = (lhs & 0xFFFF_FFFF) as u64;
    let int2_hi = (rhs as u64) >> 32;
    let int2_lo = (rhs & 0xFFFF_FFFF) as u64;
    let a = int1_hi.wrapping_mul(int2_hi);
    let b = int1_lo.wrapping_mul(int2_lo);
    let c = int1_hi.wrapping_mul(int2_lo).wrapping_add(int1_lo.wrapping_mul(int2_hi));
    let mut hi = a.wrapping_add(c >> 32) as i64;
    let lo = (c << 32).wrapping_add(b);
    if lo < b {
        hi = hi.wrapping_add(1);
    }
    if !negate {
        return (hi, lo);
    }
    if lo == 0 { (hi.wrapping_neg(), 0) } else { (!hi, (!lo).wrapping_add(1)) }
}

/// Whether `a * b == c * d`: in 64 bits (wrapping) until a coordinate
/// outgrows `loRange`, then in Clipper's 128 bits.
fn products_equal(a: i64, b: i64, c: i64, d: i64, full_range: bool) -> bool {
    if full_range {
        return int128_mul(a, b) == int128_mul(c, d);
    }
    a.wrapping_mul(b) == c.wrapping_mul(d)
}

/// `SlopesEqual` of three points.
fn slopes_equal_3(pt1: IntPoint, pt2: IntPoint, pt3: IntPoint, full_range: bool) -> bool {
    let a = pt1.y.wrapping_sub(pt2.y);
    let b = pt2.x.wrapping_sub(pt3.x);
    let c = pt1.x.wrapping_sub(pt2.x);
    let d = pt2.y.wrapping_sub(pt3.y);
    products_equal(a, b, c, d, full_range)
}

/// `SlopesEqual` of two point pairs.
fn slopes_equal_4(pt1: IntPoint, pt2: IntPoint, pt3: IntPoint, pt4: IntPoint, full_range: bool) -> bool {
    let a = pt1.y.wrapping_sub(pt2.y);
    let b = pt3.x.wrapping_sub(pt4.x);
    let c = pt1.x.wrapping_sub(pt2.x);
    let d = pt3.y.wrapping_sub(pt4.y);
    products_equal(a, b, c, d, full_range)
}

/// `SlopesEqual` of two edges.
fn slopes_equal_edges(e1: &Edge, e2: &Edge, full_range: bool) -> bool {
    let a = e1.top.y.wrapping_sub(e1.bot.y);
    let b = e2.top.x.wrapping_sub(e2.bot.x);
    let c = e1.top.x.wrapping_sub(e1.bot.x);
    let d = e2.top.y.wrapping_sub(e2.bot.y);
    products_equal(a, b, c, d, full_range)
}

/// `GetDx`: the inverse slope from `pt1` to `pt2`.
fn get_dx(pt1: IntPoint, pt2: IntPoint) -> f64 {
    if pt1.y == pt2.y {
        return HORIZONTAL;
    }
    pt2.x.wrapping_sub(pt1.x) as f64 / pt2.y.wrapping_sub(pt1.y) as f64
}

/// `TopX`: where the edge is at `current_y`.
fn top_x(edge: &Edge, current_y: i64) -> i64 {
    if current_y == edge.top.y {
        return edge.top.x;
    }
    let height = current_y.wrapping_sub(edge.bot.y) as f64;
    edge.bot.x.wrapping_add(round(edge.dx * height))
}

/// `IntersectPoint`: where two edges cross, kept inside the scanbeam.
fn intersect_point(edge_1: &Edge, edge_2: &Edge) -> IntPoint {
    let mut ip = IntPoint::default();
    if edge_1.dx == edge_2.dx {
        ip.y = edge_1.curr.y;
        ip.x = top_x(edge_1, ip.y);
        return ip;
    } else if edge_1.dx == 0.0 {
        ip.x = edge_1.bot.x;
        if edge_2.is_horizontal() {
            ip.y = edge_2.bot.y;
        } else {
            let b2 = edge_2.bot.y as f64 - edge_2.bot.x as f64 / edge_2.dx;
            ip.y = round(ip.x as f64 / edge_2.dx + b2);
        }
    } else if edge_2.dx == 0.0 {
        ip.x = edge_2.bot.x;
        if edge_1.is_horizontal() {
            ip.y = edge_1.bot.y;
        } else {
            let b1 = edge_1.bot.y as f64 - edge_1.bot.x as f64 / edge_1.dx;
            ip.y = round(ip.x as f64 / edge_1.dx + b1);
        }
    } else {
        let b1 = edge_1.bot.x as f64 - edge_1.bot.y as f64 * edge_1.dx;
        let b2 = edge_2.bot.x as f64 - edge_2.bot.y as f64 * edge_2.dx;
        let q = (b2 - b1) / (edge_1.dx - edge_2.dx);
        ip.y = round(q);
        if edge_1.dx.abs() < edge_2.dx.abs() {
            ip.x = round(edge_1.dx * q + b1);
        } else {
            ip.x = round(edge_2.dx * q + b2);
        }
    }
    if ip.y < edge_1.top.y || ip.y < edge_2.top.y {
        ip.y = if edge_1.top.y > edge_2.top.y { edge_1.top.y } else { edge_2.top.y };
        ip.x = if edge_1.dx.abs() < edge_2.dx.abs() { top_x(edge_1, ip.y) } else { top_x(edge_2, ip.y) };
    }
    if ip.y > edge_1.curr.y {
        ip.y = edge_1.curr.y;
        ip.x = if edge_1.dx.abs() > edge_2.dx.abs() { top_x(edge_2, ip.y) } else { top_x(edge_1, ip.y) };
    }
    ip
}

/// `E2InsertsBeforeE1`: AEL order.
fn e2_inserts_before_e1(e1: &Edge, e2: &Edge) -> bool {
    if e2.curr.x != e1.curr.x {
        return e2.curr.x < e1.curr.x;
    }
    if e2.top.y > e1.top.y { e2.top.x < top_x(e1, e2.top.y) } else { e1.top.x > top_x(e2, e1.top.y) }
}

/// `HorzSegmentsOverlap`.
fn horz_segments_overlap(seg1a: i64, seg1b: i64, seg2a: i64, seg2b: i64) -> bool {
    let (seg1a, seg1b) = if seg1a > seg1b { (seg1b, seg1a) } else { (seg1a, seg1b) };
    let (seg2a, seg2b) = if seg2a > seg2b { (seg2b, seg2a) } else { (seg2a, seg2b) };
    seg1a < seg2b && seg2a < seg1b
}

/// `GetOverlap`: the overlap of two ranges, if not empty.
fn get_overlap(a1: i64, a2: i64, b1: i64, b2: i64) -> Option<(i64, i64)> {
    let (left, right) = if a1 < a2 {
        if b1 < b2 { (a1.max(b1), a2.min(b2)) } else { (a1.max(b2), a2.min(b1)) }
    } else if b1 < b2 {
        (a2.max(b1), a1.min(b2))
    } else {
        (a2.max(b2), a1.min(b1))
    };
    (left < right).then_some((left, right))
}

/// `std::max` of doubles.
fn std_max(a: f64, b: f64) -> f64 {
    if a < b { b } else { a }
}

/// `std::min` of doubles.
fn std_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// `RangeTest`: past `loRange` slopes compare in 128 bits from then on;
/// past `hiRange` Clipper throws (negations wrap: `LLONG_MIN` passes).
fn range_test(point: IntPoint, use_full_range: &mut bool) -> Outcome<()> {
    let outside = |limit: i64| point.x > limit || point.y > limit || point.x.wrapping_neg() > limit || point.y.wrapping_neg() > limit;
    if !*use_full_range {
        if !outside(LOW_RANGE) {
            return Ok(());
        }
        *use_full_range = true;
    }
    if outside(HIGH_RANGE) {
        return Err(Failure::Std("Coordinate outside allowed range".to_string()));
    }
    Ok(())
}

/// `ClipperLib::Clipper`.
pub(super) struct Clipper {
    edges: Vec<Edge>,
    minima: Vec<LocalMinimum>,
    current_minimum: usize,
    use_full_range: bool,
    scanbeam: BinaryHeap<i64>,
    active_edges: Option<usize>,
    sorted_edges: Option<usize>,
    clip_type: ClipType,
    out_recs: Vec<OutRec>,
    out_pts: Vec<OutPt>,
    joins: Vec<Join>,
    ghost_joins: Vec<Join>,
    intersections: Vec<IntersectNode>,
}

impl Clipper {
    pub(super) fn new() -> Self {
        Self {
            edges: Vec::new(),
            minima: Vec::new(),
            current_minimum: 0,
            use_full_range: false,
            scanbeam: BinaryHeap::new(),
            active_edges: None,
            sorted_edges: None,
            clip_type: ClipType::Union,
            out_recs: Vec::new(),
            out_pts: Vec::new(),
            joins: Vec::new(),
            ghost_joins: Vec::new(),
            intersections: Vec::new(),
        }
    }

    /// `Clear`: the paths go. What a failed clip left in the active
    /// edges, the scanbeam and the intersections stays, as in the C++.
    pub(super) fn clear(&mut self) {
        self.minima.clear();
        self.current_minimum = 0;
        self.edges.clear();
        self.use_full_range = false;
    }

    /// `AddPaths`: whether any path was taken.
    pub(super) fn add_paths(&mut self, paths: &[Path], poly_type: PolyType) -> Outcome<bool> {
        let mut result = false;
        for path in paths {
            if self.add_path(path, poly_type)? {
                result = true;
            }
        }
        Ok(result)
    }

    /// `AddPath` of a closed path: its edges without duplicate points
    /// and collinear runs, split into bounds at each local minimum.
    /// Paths of fewer than 3 points, or flat, aren't taken.
    pub(super) fn add_path(&mut self, path: &[IntPoint], poly_type: PolyType) -> Outcome<bool> {
        let mut high = path.len() as isize - 1;
        while high > 0 && path[high as usize] == path[0] {
            high -= 1;
        }
        while high > 0 && path[high as usize] == path[high as usize - 1] {
            high -= 1;
        }
        if high < 2 {
            return Ok(false);
        }
        let high = high as usize;

        range_test(path[0], &mut self.use_full_range)?;
        range_test(path[high], &mut self.use_full_range)?;
        for index in (1..high).rev() {
            range_test(path[index], &mut self.use_full_range)?;
        }
        let base = self.edges.len();
        for (index, &point) in path[..=high].iter().enumerate() {
            let next = if index == high { base } else { base + index + 1 };
            let prev = if index == 0 { base + high } else { base + index - 1 };
            self.edges.push(Edge::new(point, next, prev));
        }

        // Duplicate points and collinear edges go.
        let mut e_start = base;
        let mut e = e_start;
        let mut e_loop_stop = e_start;
        loop {
            let next = self.edges[e].next;
            if self.edges[e].curr == self.edges[next].curr {
                if e == next {
                    break;
                }
                if e == e_start {
                    e_start = next;
                }
                e = self.remove_edge(e);
                e_loop_stop = e;
                continue;
            }
            let prev = self.edges[e].prev;
            if prev == next {
                break;
            }
            let collinear = slopes_equal_3(self.edges[prev].curr, self.edges[e].curr, self.edges[next].curr, self.use_full_range);
            if collinear {
                if e == e_start {
                    e_start = next;
                }
                let after = self.remove_edge(e);
                e = self.edges[after].prev;
                e_loop_stop = e;
                continue;
            }
            e = next;
            if e == e_loop_stop {
                break;
            }
        }
        if self.edges[e].prev == self.edges[e].next {
            self.edges.truncate(base);
            return Ok(false);
        }

        let mut is_flat = true;
        e = e_start;
        loop {
            self.init_edge_2(e, poly_type);
            e = self.edges[e].next;
            if is_flat && self.edges[e].curr.y != self.edges[e_start].curr.y {
                is_flat = false;
            }
            if e == e_start {
                break;
            }
        }
        if is_flat {
            self.edges.truncate(base);
            return Ok(false);
        }

        let mut e_min = None;
        let prev = self.edges[e].prev;
        if self.edges[prev].bot == self.edges[prev].top {
            e = self.edges[e].next;
        }
        loop {
            e = self.find_next_loc_min(e);
            if e_min == Some(e) {
                break;
            }
            if e_min.is_none() {
                e_min = Some(e);
            }
            let prev = self.edges[e].prev;
            let y = self.edges[e].bot.y;
            let (left_bound, right_bound, left_bound_is_forward) =
                if self.edges[e].dx < self.edges[prev].dx { (prev, e, false) } else { (e, prev, true) };
            let wind_delta = if self.edges[left_bound].next == right_bound { -1 } else { 1 };
            self.edges[left_bound].wind_delta = wind_delta;
            self.edges[right_bound].wind_delta = -wind_delta;

            e = self.process_bound(left_bound, left_bound_is_forward);
            let e2 = self.process_bound(right_bound, !left_bound_is_forward);
            let minimum = LocalMinimum { y, left_bound, right_bound };
            self.minima.push(minimum);
            if !left_bound_is_forward {
                e = e2;
            }
        }
        Ok(true)
    }

    /// `RemoveEdge`: unlinked; its successor.
    fn remove_edge(&mut self, e: usize) -> usize {
        let Edge { next, prev, .. } = self.edges[e];
        self.edges[prev].next = next;
        self.edges[next].prev = prev;
        next
    }

    /// `InitEdge2`: bottom, top and slope.
    fn init_edge_2(&mut self, e: usize, poly_type: PolyType) {
        let next_curr = self.edges[self.edges[e].next].curr;
        let edge = &mut self.edges[e];
        if edge.curr.y >= next_curr.y {
            edge.bot = edge.curr;
            edge.top = next_curr;
        } else {
            edge.top = edge.curr;
            edge.bot = next_curr;
        }
        let dy = edge.top.y.wrapping_sub(edge.bot.y);
        edge.dx = if dy == 0 { HORIZONTAL } else { edge.top.x.wrapping_sub(edge.bot.x) as f64 / dy as f64 };
        edge.poly_type = poly_type;
    }

    /// `ReverseHorizontal`: a horizontal's ends swapped to follow its bound.
    fn reverse_horizontal(&mut self, e: usize) {
        let edge = &mut self.edges[e];
        std::mem::swap(&mut edge.top.x, &mut edge.bot.x);
    }

    /// `FindNextLocMin`.
    fn find_next_loc_min(&self, mut e: usize) -> usize {
        let edges = &self.edges;
        loop {
            while edges[e].bot != edges[edges[e].prev].bot || edges[e].curr == edges[e].top {
                e = edges[e].next;
            }
            if !edges[e].is_horizontal() && !edges[edges[e].prev].is_horizontal() {
                break;
            }
            while edges[edges[e].prev].is_horizontal() {
                e = edges[e].prev;
            }
            let e2 = e;
            while edges[e].is_horizontal() {
                e = edges[e].next;
            }
            if edges[e].top.y == edges[edges[e].prev].bot.y {
                continue;
            }
            if edges[edges[e2].prev].bot.x < edges[e].bot.x {
                e = e2;
            }
            break;
        }
        e
    }

    /// `ProcessBound`: a bound's edges chained through `next_in_lml`; the
    /// edge just past it.
    fn process_bound(&mut self, mut e: usize, next_is_forward: bool) -> usize {
        let mut result = e;
        if self.edges[e].is_horizontal() {
            let e_start = if next_is_forward { self.edges[e].prev } else { self.edges[e].next };
            let start = self.edges[e_start];
            let bot_x = self.edges[e].bot.x;
            if start.is_horizontal() {
                if start.bot.x != bot_x && start.top.x != bot_x {
                    self.reverse_horizontal(e);
                }
            } else if start.bot.x != bot_x {
                self.reverse_horizontal(e);
            }
        }

        let e_start = e;
        if next_is_forward {
            while self.edges[result].top.y == self.edges[self.edges[result].next].bot.y {
                result = self.edges[result].next;
            }
            if self.edges[result].is_horizontal() {
                let mut horz = result;
                while self.edges[self.edges[horz].prev].is_horizontal() {
                    horz = self.edges[horz].prev;
                }
                let horz_prev = self.edges[horz].prev;
                if self.edges[horz_prev].top.x > self.edges[self.edges[result].next].top.x {
                    result = horz_prev;
                }
            }
            while e != result {
                self.edges[e].next_in_lml = Some(self.edges[e].next);
                self.reverse_inner_horizontal(e, e_start, self.edges[e].prev);
                e = self.edges[e].next;
            }
            self.reverse_inner_horizontal(e, e_start, self.edges[e].prev);
            self.edges[result].next
        } else {
            while self.edges[result].top.y == self.edges[self.edges[result].prev].bot.y {
                result = self.edges[result].prev;
            }
            if self.edges[result].is_horizontal() {
                let mut horz = result;
                while self.edges[self.edges[horz].next].is_horizontal() {
                    horz = self.edges[horz].next;
                }
                let horz_next = self.edges[horz].next;
                let result_prev_top_x = self.edges[self.edges[result].prev].top.x;
                if self.edges[horz_next].top.x >= result_prev_top_x {
                    result = horz_next;
                }
            }
            while e != result {
                self.edges[e].next_in_lml = Some(self.edges[e].prev);
                self.reverse_inner_horizontal(e, e_start, self.edges[e].next);
                e = self.edges[e].prev;
            }
            self.reverse_inner_horizontal(e, e_start, self.edges[e].next);
            self.edges[result].prev
        }
    }

    /// A horizontal past a bound's start reversed unless it starts where
    /// the edge before it in the bound ends.
    fn reverse_inner_horizontal(&mut self, e: usize, e_start: usize, before: usize) {
        if self.edges[e].is_horizontal() && e != e_start && self.edges[e].bot.x != self.edges[before].top.x {
            self.reverse_horizontal(e);
        }
    }

    /// `Reset`: minima sorted from the bottom (highest Y) up, the
    /// scanbeam refilled. Without minima nothing is reset.
    fn reset(&mut self) {
        self.current_minimum = 0;
        if self.minima.is_empty() {
            return;
        }
        msvc::sort(&mut self.minima, |a, b| b.y < a.y);
        self.scanbeam = BinaryHeap::new();
        for index in 0..self.minima.len() {
            let minimum = self.minima[index];
            self.scanbeam.push(minimum.y);
            let left = &mut self.edges[minimum.left_bound];
            left.curr = left.bot;
            left.side = Side::Left;
            left.out_index = UNASSIGNED;
            let right = &mut self.edges[minimum.right_bound];
            right.curr = right.bot;
            right.side = Side::Right;
            right.out_index = UNASSIGNED;
        }
        self.active_edges = None;
        self.current_minimum = 0;
    }

    fn pop_local_minimum(&mut self, y: i64) -> Option<LocalMinimum> {
        let minimum = *self.minima.get(self.current_minimum)?;
        if minimum.y != y {
            return None;
        }
        self.current_minimum += 1;
        Some(minimum)
    }

    /// `PopScanbeam`: the highest Y left, its duplicates dropped.
    fn pop_scanbeam(&mut self) -> Option<i64> {
        let y = self.scanbeam.pop()?;
        while self.scanbeam.peek() == Some(&y) {
            self.scanbeam.pop();
        }
        Some(y)
    }

    /// `Execute` into paths, both polygon kinds filled non-zero. A clip
    /// that fails gives none.
    pub(super) fn execute(&mut self, clip_type: ClipType) -> Outcome<Vec<Path>> {
        self.clip_type = clip_type;
        let succeeded = self.execute_internal()?;
        let solution = if succeeded { self.build_result() } else { Vec::new() };
        self.out_recs.clear();
        self.out_pts.clear();
        Ok(solution)
    }

    /// `ExecuteInternal`: the sweep (Clipper's exceptions in it are
    /// caught), then orientations, joins and clean-up.
    fn execute_internal(&mut self) -> Outcome<bool> {
        let succeeded = match self.sweep() {
            Ok(succeeded) => succeeded,
            Err(Failure::Std(_)) => false,
            Err(failure) => return Err(failure),
        };
        if succeeded {
            for index in 0..self.out_recs.len() {
                let Some(points) = self.out_recs[index].points else {
                    continue;
                };
                if self.out_recs[index].is_hole == (self.area_of(points) > 0.0) {
                    self.reverse_poly_pt_links(points);
                }
            }
            if !self.joins.is_empty() {
                self.join_common_edges()?;
            }
            for index in 0..self.out_recs.len() {
                if self.out_recs[index].points.is_some() {
                    self.fixup_out_polygon(index);
                }
            }
        }
        self.joins.clear();
        self.ghost_joins.clear();
        Ok(succeeded)
    }

    /// The scanbeam loop of `ExecuteInternal`.
    fn sweep(&mut self) -> Outcome<bool> {
        self.reset();
        self.sorted_edges = None;
        let Some(mut bot_y) = self.pop_scanbeam() else {
            return Ok(false);
        };
        self.insert_local_minima_into_ael(bot_y)?;
        let mut top_y = bot_y;
        loop {
            match self.pop_scanbeam() {
                Some(y) => top_y = y,
                None if self.current_minimum < self.minima.len() => {}
                None => break,
            }
            self.process_horizontals()?;
            self.ghost_joins.clear();
            if !self.process_intersections(top_y)? {
                return Ok(false);
            }
            self.process_edges_at_top_of_scanbeam(top_y)?;
            bot_y = top_y;
            self.insert_local_minima_into_ael(bot_y)?;
        }
        Ok(true)
    }

    /// `InsertLocalMinimaIntoAEL`: the bounds starting at `bot_y` join
    /// the active edges.
    fn insert_local_minima_into_ael(&mut self, bot_y: i64) -> Outcome<()> {
        while let Some(minimum) = self.pop_local_minimum(bot_y) {
            let lb = minimum.left_bound;
            let rb = minimum.right_bound;
            self.insert_edge_into_ael(lb, None);
            self.insert_edge_into_ael(rb, Some(lb));
            self.set_winding_count(lb)?;
            self.edges[rb].wind_count = self.edges[lb].wind_count;
            self.edges[rb].wind_count_2 = self.edges[lb].wind_count_2;
            let mut op1 = None;
            if self.is_contributing(lb) {
                let bot = self.edges[lb].bot;
                op1 = Some(self.add_local_min_poly(lb, rb, bot)?);
            }
            self.scanbeam.push(self.edges[lb].top.y);

            if self.edges[rb].is_horizontal() {
                self.add_edge_to_sel(rb);
                if let Some(next) = self.edges[rb].next_in_lml {
                    self.scanbeam.push(self.edges[next].top.y);
                }
            } else {
                self.scanbeam.push(self.edges[rb].top.y);
            }

            // Output polygons sharing an edge get joined later.
            if let Some(op1) = op1
                && self.edges[rb].is_horizontal()
            {
                for index in 0..self.ghost_joins.len() {
                    let ghost = self.ghost_joins[index];
                    let ghost_x = self.out_pts[self.out_point(ghost.out_pt_1)?].point.x;
                    let rb_edge = self.edges[rb];
                    if horz_segments_overlap(ghost_x, ghost.offset_point.x, rb_edge.bot.x, rb_edge.top.x) {
                        self.add_join(ghost.out_pt_1, Some(op1), ghost.offset_point);
                    }
                }
            }

            let lb_edge = self.edges[lb];
            if lb_edge.out_index >= 0
                && let Some(lb_prev) = lb_edge.prev_in_ael
                && self.edges[lb_prev].curr.x == lb_edge.bot.x
                && self.edges[lb_prev].out_index >= 0
                && slopes_equal_4(self.edges[lb_prev].bot, self.edges[lb_prev].top, lb_edge.curr, lb_edge.top, self.use_full_range)
            {
                let op2 = self.add_out_pt(lb_prev, lb_edge.bot)?;
                self.add_join(op1, Some(op2), lb_edge.top);
            }

            if self.edges[lb].next_in_ael != Some(rb) {
                let rb_edge = self.edges[rb];
                let rb_prev = self.ael_link(rb_edge.prev_in_ael)?;
                if rb_edge.out_index >= 0
                    && self.edges[rb_prev].out_index >= 0
                    && slopes_equal_4(self.edges[rb_prev].curr, self.edges[rb_prev].top, rb_edge.curr, rb_edge.top, self.use_full_range)
                {
                    let op2 = self.add_out_pt(rb_prev, rb_edge.bot)?;
                    self.add_join(op1, Some(op2), rb_edge.top);
                }

                if let Some(mut e) = self.edges[lb].next_in_ael {
                    while e != rb {
                        // The right bound is to the right above the crossing.
                        let point = self.edges[lb].curr;
                        self.intersect_edges(rb, e, point)?;
                        e = self.ael_link(self.edges[e].next_in_ael)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// An AEL link the C++ follows without checking.
    fn ael_link(&self, link: Option<usize>) -> Outcome<usize> {
        link.ok_or_else(|| Failure::undefined("Clipper followed a missing active edge"))
    }

    /// An output point the C++ follows without checking.
    fn out_point(&self, point: Option<usize>) -> Outcome<usize> {
        point.ok_or_else(|| Failure::undefined("Clipper followed a missing output point"))
    }

    /// An output polygon's points (none once merged into another).
    fn points_of(&self, out_rec: usize) -> Outcome<usize> {
        self.out_point(self.out_recs[out_rec].points)
    }

    /// `InsertEdgeIntoAEL`: by `E2InsertsBeforeE1`, searching from
    /// `start_edge` when given.
    fn insert_edge_into_ael(&mut self, edge: usize, start_edge: Option<usize>) {
        let Some(first) = self.active_edges else {
            self.edges[edge].prev_in_ael = None;
            self.edges[edge].next_in_ael = None;
            self.active_edges = Some(edge);
            return;
        };
        if start_edge.is_none() && e2_inserts_before_e1(&self.edges[first], &self.edges[edge]) {
            self.edges[edge].prev_in_ael = None;
            self.edges[edge].next_in_ael = Some(first);
            self.edges[first].prev_in_ael = Some(edge);
            self.active_edges = Some(edge);
            return;
        }
        let mut start = start_edge.unwrap_or(first);
        while let Some(next) = self.edges[start].next_in_ael
            && !e2_inserts_before_e1(&self.edges[next], &self.edges[edge])
        {
            start = next;
        }
        let after = self.edges[start].next_in_ael;
        self.edges[edge].next_in_ael = after;
        if let Some(after) = after {
            self.edges[after].prev_in_ael = Some(edge);
        }
        self.edges[edge].prev_in_ael = Some(start);
        self.edges[start].next_in_ael = Some(edge);
    }

    /// `SetWindingCount` with non-zero filling.
    fn set_winding_count(&mut self, edge: usize) -> Outcome<()> {
        let poly_type = self.edges[edge].poly_type;
        let wind_delta = self.edges[edge].wind_delta;
        let mut e = self.edges[edge].prev_in_ael;
        while let Some(index) = e
            && self.edges[index].poly_type != poly_type
        {
            e = self.edges[index].prev_in_ael;
        }
        let mut walk = match e {
            None => {
                self.edges[edge].wind_count = wind_delta;
                self.edges[edge].wind_count_2 = 0;
                self.active_edges
            }
            Some(index) => {
                let before = self.edges[index];
                let wind_count = if before.wind_count * before.wind_delta < 0 {
                    // Outside the polygon before…
                    if (before.wind_count as i64).abs() > 1 {
                        // …but still inside another.
                        if before.wind_delta * wind_delta < 0 { before.wind_count } else { before.wind_count + wind_delta }
                    } else {
                        wind_delta
                    }
                } else if before.wind_delta * wind_delta < 0 {
                    before.wind_count
                } else {
                    before.wind_count + wind_delta
                };
                self.edges[edge].wind_count = wind_count;
                self.edges[edge].wind_count_2 = before.wind_count_2;
                before.next_in_ael
            }
        };
        while walk != Some(edge) {
            let index = self.ael_link(walk)?;
            self.edges[edge].wind_count_2 += self.edges[index].wind_delta;
            walk = self.edges[index].next_in_ael;
        }
        Ok(())
    }

    /// `IsContributing` with non-zero filling.
    fn is_contributing(&self, edge: usize) -> bool {
        let edge = &self.edges[edge];
        if (edge.wind_count as i64).abs() != 1 {
            return false;
        }
        match self.clip_type {
            ClipType::Intersection => edge.wind_count_2 != 0,
            ClipType::Union => edge.wind_count_2 == 0,
            ClipType::Difference => {
                if edge.poly_type == PolyType::Subject {
                    edge.wind_count_2 == 0
                } else {
                    edge.wind_count_2 != 0
                }
            }
        }
    }

    /// `AddLocalMinPoly`: a new output polygon's first point.
    fn add_local_min_poly(&mut self, e1: usize, e2: usize, point: IntPoint) -> Outcome<usize> {
        // The left edge of the new polygon and its other edge.
        let (e, other) = if self.edges[e2].is_horizontal() || self.edges[e1].dx > self.edges[e2].dx { (e1, e2) } else { (e2, e1) };
        let result = self.add_out_pt(e, point)?;
        self.edges[other].out_index = self.edges[e].out_index;
        self.edges[e].side = Side::Left;
        self.edges[other].side = Side::Right;
        let prev_e = if self.edges[e].prev_in_ael == Some(other) { self.edges[other].prev_in_ael } else { self.edges[e].prev_in_ael };

        if let Some(prev_e) = prev_e
            && self.edges[prev_e].out_index >= 0
            && self.edges[prev_e].top.y < point.y
            && self.edges[e].top.y < point.y
        {
            let x_prev = top_x(&self.edges[prev_e], point.y);
            let x_e = top_x(&self.edges[e], point.y);
            let prev_top = self.edges[prev_e].top;
            let e_top = self.edges[e].top;
            if x_prev == x_e
                && slopes_equal_4(IntPoint::new(x_prev, point.y), prev_top, IntPoint::new(x_e, point.y), e_top, self.use_full_range)
            {
                let out_pt = self.add_out_pt(prev_e, point)?;
                self.add_join(Some(result), Some(out_pt), e_top);
            }
        }
        Ok(result)
    }

    /// `AddLocalMaxPoly`: the two sides of output polygons meet.
    fn add_local_max_poly(&mut self, e1: usize, e2: usize, point: IntPoint) -> Outcome<()> {
        self.add_out_pt(e1, point)?;
        let index_1 = self.edges[e1].out_index;
        let index_2 = self.edges[e2].out_index;
        if index_1 == index_2 {
            self.edges[e1].out_index = UNASSIGNED;
            self.edges[e2].out_index = UNASSIGNED;
        } else if index_1 < index_2 {
            self.append_polygon(e1, e2)?;
        } else {
            self.append_polygon(e2, e1)?;
        }
        Ok(())
    }

    /// `AddEdgeToSEL`: at the front.
    fn add_edge_to_sel(&mut self, edge: usize) {
        match self.sorted_edges {
            None => {
                self.edges[edge].prev_in_sel = None;
                self.edges[edge].next_in_sel = None;
            }
            Some(first) => {
                self.edges[edge].next_in_sel = Some(first);
                self.edges[edge].prev_in_sel = None;
                self.edges[first].prev_in_sel = Some(edge);
            }
        }
        self.sorted_edges = Some(edge);
    }

    fn pop_edge_from_sel(&mut self) -> Option<usize> {
        let edge = self.sorted_edges?;
        self.delete_from_sel(edge);
        Some(edge)
    }

    /// `CopyAELToSEL`.
    fn copy_ael_to_sel(&mut self) {
        let mut e = self.active_edges;
        self.sorted_edges = e;
        while let Some(index) = e {
            self.edges[index].prev_in_sel = self.edges[index].prev_in_ael;
            self.edges[index].next_in_sel = self.edges[index].next_in_ael;
            e = self.edges[index].next_in_ael;
        }
    }

    fn add_join(&mut self, out_pt_1: Option<usize>, out_pt_2: Option<usize>, offset_point: IntPoint) {
        let join = Join { out_pt_1, out_pt_2, offset_point };
        self.joins.push(join);
    }

    fn add_ghost_join(&mut self, out_pt: usize, offset_point: IntPoint) {
        let join = Join { out_pt_1: Some(out_pt), out_pt_2: None, offset_point };
        self.ghost_joins.push(join);
    }

    /// `DeleteFromAEL` (an edge already out stays out).
    fn delete_from_ael(&mut self, e: usize) {
        let prev = self.edges[e].prev_in_ael;
        let next = self.edges[e].next_in_ael;
        if prev.is_none() && next.is_none() && self.active_edges != Some(e) {
            return;
        }
        match prev {
            Some(prev) => self.edges[prev].next_in_ael = next,
            None => self.active_edges = next,
        }
        if let Some(next) = next {
            self.edges[next].prev_in_ael = prev;
        }
        self.edges[e].next_in_ael = None;
        self.edges[e].prev_in_ael = None;
    }

    /// `DeleteFromSEL`.
    fn delete_from_sel(&mut self, e: usize) {
        let prev = self.edges[e].prev_in_sel;
        let next = self.edges[e].next_in_sel;
        if prev.is_none() && next.is_none() && self.sorted_edges != Some(e) {
            return;
        }
        match prev {
            Some(prev) => self.edges[prev].next_in_sel = next,
            None => self.sorted_edges = next,
        }
        if let Some(next) = next {
            self.edges[next].prev_in_sel = prev;
        }
        self.edges[e].next_in_sel = None;
        self.edges[e].prev_in_sel = None;
    }

    /// `IntersectEdges` of closed edges filled non-zero: winding counts
    /// updated, output polygons started, continued or closed. `e1` is to
    /// the right of `e2` above the crossing.
    fn intersect_edges(&mut self, e1: usize, e2: usize, point: IntPoint) -> Outcome<()> {
        let e1_contributing = self.edges[e1].out_index >= 0;
        let e2_contributing = self.edges[e2].out_index >= 0;

        let e1_delta = self.edges[e1].wind_delta;
        let e2_delta = self.edges[e2].wind_delta;
        if self.edges[e1].poly_type == self.edges[e2].poly_type {
            let edge_1 = &mut self.edges[e1];
            if edge_1.wind_count + e2_delta == 0 {
                edge_1.wind_count = -edge_1.wind_count;
            } else {
                edge_1.wind_count += e2_delta;
            }
            let edge_2 = &mut self.edges[e2];
            if edge_2.wind_count - e1_delta == 0 {
                edge_2.wind_count = -edge_2.wind_count;
            } else {
                edge_2.wind_count -= e1_delta;
            }
        } else {
            self.edges[e1].wind_count_2 += e2_delta;
            self.edges[e2].wind_count_2 -= e1_delta;
        }

        let e1_wc = (self.edges[e1].wind_count as i64).abs();
        let e2_wc = (self.edges[e2].wind_count as i64).abs();
        let e1_wc_is_0_or_1 = e1_wc == 0 || e1_wc == 1;
        let e2_wc_is_0_or_1 = e2_wc == 0 || e2_wc == 1;
        let same_type = self.edges[e1].poly_type == self.edges[e2].poly_type;

        if e1_contributing && e2_contributing {
            if !e1_wc_is_0_or_1 || !e2_wc_is_0_or_1 || !same_type {
                self.add_local_max_poly(e1, e2, point)?;
            } else {
                self.add_out_pt(e1, point)?;
                self.add_out_pt(e2, point)?;
                self.swap_sides(e1, e2);
                self.swap_poly_indexes(e1, e2);
            }
        } else if e1_contributing {
            if e2_wc_is_0_or_1 {
                self.add_out_pt(e1, point)?;
                self.swap_sides(e1, e2);
                self.swap_poly_indexes(e1, e2);
            }
        } else if e2_contributing {
            if e1_wc_is_0_or_1 {
                self.add_out_pt(e2, point)?;
                self.swap_sides(e1, e2);
                self.swap_poly_indexes(e1, e2);
            }
        } else if e1_wc_is_0_or_1 && e2_wc_is_0_or_1 {
            // Neither edge contributes yet.
            let e1_wc2 = (self.edges[e1].wind_count_2 as i64).abs();
            let e2_wc2 = (self.edges[e2].wind_count_2 as i64).abs();
            if !same_type {
                self.add_local_min_poly(e1, e2, point)?;
            } else if e1_wc == 1 && e2_wc == 1 {
                let starts = match self.clip_type {
                    ClipType::Intersection => e1_wc2 > 0 && e2_wc2 > 0,
                    ClipType::Union => e1_wc2 <= 0 && e2_wc2 <= 0,
                    ClipType::Difference => {
                        let e1_type = self.edges[e1].poly_type;
                        (e1_type == PolyType::Clip && e1_wc2 > 0 && e2_wc2 > 0)
                            || (e1_type == PolyType::Subject && e1_wc2 <= 0 && e2_wc2 <= 0)
                    }
                };
                if starts {
                    self.add_local_min_poly(e1, e2, point)?;
                }
            } else {
                self.swap_sides(e1, e2);
            }
        }
        Ok(())
    }

    fn swap_sides(&mut self, e1: usize, e2: usize) {
        let side = self.edges[e1].side;
        self.edges[e1].side = self.edges[e2].side;
        self.edges[e2].side = side;
    }

    fn swap_poly_indexes(&mut self, e1: usize, e2: usize) {
        let out_index = self.edges[e1].out_index;
        self.edges[e1].out_index = self.edges[e2].out_index;
        self.edges[e2].out_index = out_index;
    }

    /// `SetHoleState`: a new polygon is a hole when an odd number of
    /// output polygons lie to its left.
    fn set_hole_state(&mut self, e: usize, out_rec: usize) {
        let mut e2 = self.edges[e].prev_in_ael;
        let mut e_tmp: Option<usize> = None;
        while let Some(index) = e2 {
            let out_index = self.edges[index].out_index;
            if out_index >= 0 {
                match e_tmp {
                    None => e_tmp = Some(index),
                    Some(tmp) if self.edges[tmp].out_index == out_index => e_tmp = None,
                    Some(_) => {}
                }
            }
            e2 = self.edges[index].prev_in_ael;
        }
        match e_tmp {
            None => {
                self.out_recs[out_rec].first_left = None;
                self.out_recs[out_rec].is_hole = false;
            }
            Some(tmp) => {
                let first_left = self.edges[tmp].out_index as usize;
                self.out_recs[out_rec].first_left = Some(first_left);
                self.out_recs[out_rec].is_hole = !self.out_recs[first_left].is_hole;
            }
        }
    }

    /// `GetLowermostRec`: which of two polygons has the right hole state.
    fn get_lowermost_rec(&mut self, out_rec_1: usize, out_rec_2: usize) -> Outcome<usize> {
        if self.out_recs[out_rec_1].bottom_point.is_none() {
            let points = self.points_of(out_rec_1)?;
            self.out_recs[out_rec_1].bottom_point = Some(self.get_bottom_pt(points));
        }
        if self.out_recs[out_rec_2].bottom_point.is_none() {
            let points = self.points_of(out_rec_2)?;
            self.out_recs[out_rec_2].bottom_point = Some(self.get_bottom_pt(points));
        }
        let out_pt_1 = self.out_point(self.out_recs[out_rec_1].bottom_point)?;
        let out_pt_2 = self.out_point(self.out_recs[out_rec_2].bottom_point)?;
        let point_1 = self.out_pts[out_pt_1].point;
        let point_2 = self.out_pts[out_pt_2].point;
        let lowermost = if point_1.y > point_2.y {
            out_rec_1
        } else if point_1.y < point_2.y {
            out_rec_2
        } else if point_1.x < point_2.x {
            out_rec_1
        } else if point_1.x > point_2.x || self.out_pts[out_pt_1].next == out_pt_1 {
            out_rec_2
        } else if self.out_pts[out_pt_2].next == out_pt_2 || self.first_is_bottom_pt(out_pt_1, out_pt_2) {
            out_rec_1
        } else {
            out_rec_2
        };
        Ok(lowermost)
    }

    /// `OutRec1RightOfOutRec2`: whether `out_rec_2` is among the
    /// polygons left of `out_rec_1`.
    fn out_rec_1_right_of_out_rec_2(&self, out_rec_1: usize, out_rec_2: usize) -> Outcome<bool> {
        let mut current = Some(out_rec_1);
        let mut steps = 0usize;
        while let Some(index) = current {
            current = self.out_recs[index].first_left;
            if current == Some(out_rec_2) {
                return Ok(true);
            }
            steps += 1;
            if steps > self.out_recs.len() {
                return Err(Failure::endless("Clipper's polygon owners form a cycle"));
            }
        }
        Ok(false)
    }

    /// `GetOutRec`: the polygon an index ends up in after merges.
    fn get_out_rec(&self, index: i32) -> Outcome<usize> {
        let mut out_rec = index as usize;
        let mut steps = 0usize;
        while out_rec != self.out_recs[out_rec].index as usize {
            out_rec = self.out_recs[out_rec].index as usize;
            steps += 1;
            if steps > self.out_recs.len() {
                return Err(Failure::endless("Clipper's merged polygons form a cycle"));
            }
        }
        Ok(out_rec)
    }

    /// `AppendPolygon`: `e2`'s polygon joined onto `e1`'s.
    fn append_polygon(&mut self, e1: usize, e2: usize) -> Outcome<()> {
        let out_rec_1 = self.edges[e1].out_index as usize;
        let out_rec_2 = self.edges[e2].out_index as usize;

        let hole_state_rec = if self.out_rec_1_right_of_out_rec_2(out_rec_1, out_rec_2)? {
            out_rec_2
        } else if self.out_rec_1_right_of_out_rec_2(out_rec_2, out_rec_1)? {
            out_rec_1
        } else {
            self.get_lowermost_rec(out_rec_1, out_rec_2)?
        };

        let p1_lft = self.points_of(out_rec_1)?;
        let p1_rt = self.out_pts[p1_lft].prev;
        let p2_lft = self.points_of(out_rec_2)?;
        let p2_rt = self.out_pts[p2_lft].prev;

        let e1_side = self.edges[e1].side;
        let e2_side = self.edges[e2].side;
        if e1_side == Side::Left {
            if e2_side == Side::Left {
                // z y x a b c
                self.reverse_poly_pt_links(p2_lft);
                self.out_pts[p2_lft].next = p1_lft;
                self.out_pts[p1_lft].prev = p2_lft;
                self.out_pts[p1_rt].next = p2_rt;
                self.out_pts[p2_rt].prev = p1_rt;
                self.out_recs[out_rec_1].points = Some(p2_rt);
            } else {
                // x y z a b c
                self.out_pts[p2_rt].next = p1_lft;
                self.out_pts[p1_lft].prev = p2_rt;
                self.out_pts[p2_lft].prev = p1_rt;
                self.out_pts[p1_rt].next = p2_lft;
                self.out_recs[out_rec_1].points = Some(p2_lft);
            }
        } else if e2_side == Side::Right {
            // a b c z y x
            self.reverse_poly_pt_links(p2_lft);
            self.out_pts[p1_rt].next = p2_rt;
            self.out_pts[p2_rt].prev = p1_rt;
            self.out_pts[p2_lft].next = p1_lft;
            self.out_pts[p1_lft].prev = p2_lft;
        } else {
            // a b c x y z
            self.out_pts[p1_rt].next = p2_lft;
            self.out_pts[p2_lft].prev = p1_rt;
            self.out_pts[p1_lft].prev = p2_rt;
            self.out_pts[p2_rt].next = p1_lft;
        }

        self.out_recs[out_rec_1].bottom_point = None;
        if hole_state_rec == out_rec_2 {
            if self.out_recs[out_rec_2].first_left != Some(out_rec_1) {
                self.out_recs[out_rec_1].first_left = self.out_recs[out_rec_2].first_left;
            }
            self.out_recs[out_rec_1].is_hole = self.out_recs[out_rec_2].is_hole;
        }
        self.out_recs[out_rec_2].points = None;
        self.out_recs[out_rec_2].bottom_point = None;
        self.out_recs[out_rec_2].first_left = Some(out_rec_1);

        let ok_index = self.edges[e1].out_index;
        let obsolete_index = self.edges[e2].out_index;
        self.edges[e1].out_index = UNASSIGNED;
        self.edges[e2].out_index = UNASSIGNED;

        let mut e = self.active_edges;
        while let Some(index) = e {
            if self.edges[index].out_index == obsolete_index {
                self.edges[index].out_index = ok_index;
                self.edges[index].side = e1_side;
                break;
            }
            e = self.edges[index].next_in_ael;
        }
        self.out_recs[out_rec_2].index = self.out_recs[out_rec_1].index;
        Ok(())
    }

    /// `AddOutPt`: a point on the edge's side of its output polygon (a
    /// new polygon when it has none). A point equal to that end isn't
    /// repeated.
    fn add_out_pt(&mut self, e: usize, point: IntPoint) -> Outcome<usize> {
        let out_index = self.edges[e].out_index;
        if out_index < 0 {
            let out_rec = self.out_recs.len();
            let new_op = self.out_pts.len();
            let record = OutRec { index: out_rec as i32, is_hole: false, first_left: None, points: Some(new_op), bottom_point: None };
            self.out_recs.push(record);
            let out_pt = OutPt { index: out_rec as i32, point, next: new_op, prev: new_op };
            self.out_pts.push(out_pt);
            self.set_hole_state(e, out_rec);
            self.edges[e].out_index = out_rec as i32;
            return Ok(new_op);
        }

        let out_rec = out_index as usize;
        let op = self.points_of(out_rec)?;
        let op_prev = self.out_pts[op].prev;
        let to_front = self.edges[e].side == Side::Left;
        if to_front && point == self.out_pts[op].point {
            return Ok(op);
        }
        if !to_front && point == self.out_pts[op_prev].point {
            return Ok(op_prev);
        }
        let new_op = self.out_pts.len();
        let out_pt = OutPt { index: self.out_recs[out_rec].index, point, next: op, prev: op_prev };
        self.out_pts.push(out_pt);
        self.out_pts[op_prev].next = new_op;
        self.out_pts[op].prev = new_op;
        if to_front {
            self.out_recs[out_rec].points = Some(new_op);
        }
        Ok(new_op)
    }

    /// `GetLastOutPt`: the end of the edge's output polygon on its side.
    fn get_last_out_pt(&self, e: usize) -> Outcome<usize> {
        let points = self.points_of(self.edges[e].out_index as usize)?;
        if self.edges[e].side == Side::Left { Ok(points) } else { Ok(self.out_pts[points].prev) }
    }

    fn process_horizontals(&mut self) -> Outcome<()> {
        while let Some(horz_edge) = self.pop_edge_from_sel() {
            self.process_horizontal(horz_edge)?;
        }
        Ok(())
    }

    /// `GetMaximaPair`: the edge meeting this one at its top.
    fn get_maxima_pair(&self, e: usize) -> Option<usize> {
        let edge = &self.edges[e];
        let next = &self.edges[edge.next];
        if next.top == edge.top && next.next_in_lml.is_none() {
            return Some(edge.next);
        }
        let prev = &self.edges[edge.prev];
        if prev.top == edge.top && prev.next_in_lml.is_none() {
            return Some(edge.prev);
        }
        None
    }

    /// `GetMaximaPairEx`: the maxima pair if it's active (or horizontal).
    fn get_maxima_pair_ex(&self, e: usize) -> Option<usize> {
        let result = self.get_maxima_pair(e)?;
        let pair = &self.edges[result];
        if pair.next_in_ael == pair.prev_in_ael && !pair.is_horizontal() {
            return None;
        }
        Some(result)
    }

    /// `SwapPositionsInAEL` (not when either is out of the list).
    fn swap_positions_in_ael(&mut self, edge_1: usize, edge_2: usize) {
        let first = self.edges[edge_1];
        let second = self.edges[edge_2];
        if first.next_in_ael == first.prev_in_ael || second.next_in_ael == second.prev_in_ael {
            return;
        }
        if first.next_in_ael == Some(edge_2) {
            let next = second.next_in_ael;
            if let Some(next) = next {
                self.edges[next].prev_in_ael = Some(edge_1);
            }
            let prev = first.prev_in_ael;
            if let Some(prev) = prev {
                self.edges[prev].next_in_ael = Some(edge_2);
            }
            self.edges[edge_2].prev_in_ael = prev;
            self.edges[edge_2].next_in_ael = Some(edge_1);
            self.edges[edge_1].prev_in_ael = Some(edge_2);
            self.edges[edge_1].next_in_ael = next;
        } else if second.next_in_ael == Some(edge_1) {
            let next = first.next_in_ael;
            if let Some(next) = next {
                self.edges[next].prev_in_ael = Some(edge_2);
            }
            let prev = second.prev_in_ael;
            if let Some(prev) = prev {
                self.edges[prev].next_in_ael = Some(edge_1);
            }
            self.edges[edge_1].prev_in_ael = prev;
            self.edges[edge_1].next_in_ael = Some(edge_2);
            self.edges[edge_2].prev_in_ael = Some(edge_1);
            self.edges[edge_2].next_in_ael = next;
        } else {
            let next = first.next_in_ael;
            let prev = first.prev_in_ael;
            self.edges[edge_1].next_in_ael = second.next_in_ael;
            if let Some(after) = second.next_in_ael {
                self.edges[after].prev_in_ael = Some(edge_1);
            }
            self.edges[edge_1].prev_in_ael = second.prev_in_ael;
            if let Some(before) = second.prev_in_ael {
                self.edges[before].next_in_ael = Some(edge_1);
            }
            self.edges[edge_2].next_in_ael = next;
            if let Some(after) = next {
                self.edges[after].prev_in_ael = Some(edge_2);
            }
            self.edges[edge_2].prev_in_ael = prev;
            if let Some(before) = prev {
                self.edges[before].next_in_ael = Some(edge_2);
            }
        }
        if self.edges[edge_1].prev_in_ael.is_none() {
            self.active_edges = Some(edge_1);
        } else if self.edges[edge_2].prev_in_ael.is_none() {
            self.active_edges = Some(edge_2);
        }
    }

    /// `SwapPositionsInSEL` (not when either is out of the list).
    fn swap_positions_in_sel(&mut self, edge_1: usize, edge_2: usize) {
        let first = self.edges[edge_1];
        let second = self.edges[edge_2];
        if first.next_in_sel.is_none() && first.prev_in_sel.is_none() {
            return;
        }
        if second.next_in_sel.is_none() && second.prev_in_sel.is_none() {
            return;
        }
        if first.next_in_sel == Some(edge_2) {
            let next = second.next_in_sel;
            if let Some(next) = next {
                self.edges[next].prev_in_sel = Some(edge_1);
            }
            let prev = first.prev_in_sel;
            if let Some(prev) = prev {
                self.edges[prev].next_in_sel = Some(edge_2);
            }
            self.edges[edge_2].prev_in_sel = prev;
            self.edges[edge_2].next_in_sel = Some(edge_1);
            self.edges[edge_1].prev_in_sel = Some(edge_2);
            self.edges[edge_1].next_in_sel = next;
        } else if second.next_in_sel == Some(edge_1) {
            let next = first.next_in_sel;
            if let Some(next) = next {
                self.edges[next].prev_in_sel = Some(edge_2);
            }
            let prev = second.prev_in_sel;
            if let Some(prev) = prev {
                self.edges[prev].next_in_sel = Some(edge_1);
            }
            self.edges[edge_1].prev_in_sel = prev;
            self.edges[edge_1].next_in_sel = Some(edge_2);
            self.edges[edge_2].prev_in_sel = Some(edge_1);
            self.edges[edge_2].next_in_sel = next;
        } else {
            let next = first.next_in_sel;
            let prev = first.prev_in_sel;
            self.edges[edge_1].next_in_sel = second.next_in_sel;
            if let Some(after) = second.next_in_sel {
                self.edges[after].prev_in_sel = Some(edge_1);
            }
            self.edges[edge_1].prev_in_sel = second.prev_in_sel;
            if let Some(before) = second.prev_in_sel {
                self.edges[before].next_in_sel = Some(edge_1);
            }
            self.edges[edge_2].next_in_sel = next;
            if let Some(after) = next {
                self.edges[after].prev_in_sel = Some(edge_2);
            }
            self.edges[edge_2].prev_in_sel = prev;
            if let Some(before) = prev {
                self.edges[before].next_in_sel = Some(edge_2);
            }
        }
        if self.edges[edge_1].prev_in_sel.is_none() {
            self.sorted_edges = Some(edge_1);
        } else if self.edges[edge_2].prev_in_sel.is_none() {
            self.sorted_edges = Some(edge_2);
        }
    }

    fn get_next_in_ael(&self, e: usize, direction: Direction) -> Option<usize> {
        if direction == Direction::LeftToRight { self.edges[e].next_in_ael } else { self.edges[e].prev_in_ael }
    }

    /// `GetHorzDirection`: which way a horizontal runs, and its ends.
    fn get_horz_direction(&self, e: usize) -> (Direction, i64, i64) {
        let edge = &self.edges[e];
        if edge.bot.x < edge.top.x {
            (Direction::LeftToRight, edge.bot.x, edge.top.x)
        } else {
            (Direction::RightToLeft, edge.top.x, edge.bot.x)
        }
    }

    /// `UpdateEdgeIntoAEL`: the next edge of the bound takes the edge's
    /// place. Past the bound's top Clipper throws.
    fn update_edge_into_ael(&mut self, e: &mut usize) -> Outcome<()> {
        let Some(next) = self.edges[*e].next_in_lml else {
            return Err(Failure::Std("UpdateEdgeIntoAEL: invalid call".to_string()));
        };
        let old = self.edges[*e];
        self.edges[next].out_index = old.out_index;
        match old.prev_in_ael {
            Some(prev) => self.edges[prev].next_in_ael = Some(next),
            None => self.active_edges = Some(next),
        }
        if let Some(after) = old.next_in_ael {
            self.edges[after].prev_in_ael = Some(next);
        }
        let edge = &mut self.edges[next];
        edge.side = old.side;
        edge.wind_delta = old.wind_delta;
        edge.wind_count = old.wind_count;
        edge.wind_count_2 = old.wind_count_2;
        edge.curr = edge.bot;
        edge.prev_in_ael = old.prev_in_ael;
        edge.next_in_ael = old.next_in_ael;
        if !edge.is_horizontal() {
            let top_y = edge.top.y;
            self.scanbeam.push(top_y);
        }
        *e = next;
        Ok(())
    }

    /// Joins from a horizontal's output point to the output of each
    /// queued horizontal it overlaps.
    fn join_overlapping_horizontals(&mut self, horz_edge: usize, op1: usize) -> Outcome<()> {
        let horz = self.edges[horz_edge];
        let mut e_next_horz = self.sorted_edges;
        while let Some(next_horz) = e_next_horz {
            let other = self.edges[next_horz];
            if other.out_index >= 0 && horz_segments_overlap(horz.bot.x, horz.top.x, other.bot.x, other.top.x) {
                let op2 = self.get_last_out_pt(next_horz)?;
                self.add_join(Some(op2), Some(op1), other.top);
            }
            e_next_horz = other.next_in_sel;
        }
        Ok(())
    }

    /// `ProcessHorizontal`: a horizontal (and the horizontals following it
    /// in its bound) crossing the edges along it.
    fn process_horizontal(&mut self, mut horz_edge: usize) -> Outcome<()> {
        let (mut direction, mut horz_left, mut horz_right) = self.get_horz_direction(horz_edge);

        let mut e_last_horz = horz_edge;
        while let Some(next) = self.edges[e_last_horz].next_in_lml
            && self.edges[next].is_horizontal()
        {
            e_last_horz = next;
        }
        let e_max_pair = if self.edges[e_last_horz].next_in_lml.is_none() { self.get_maxima_pair(e_last_horz) } else { None };

        let mut op1 = None;
        loop {
            let is_last_horz = horz_edge == e_last_horz;
            let mut e = self.get_next_in_ael(horz_edge, direction);
            while let Some(current) = e {
                let current_x = self.edges[current].curr.x;
                if (direction == Direction::LeftToRight && current_x > horz_right) || (direction == Direction::RightToLeft && current_x < horz_left) {
                    break;
                }
                // The end of an intermediate horizontal: smaller slopes are
                // to the right above it.
                if current_x == self.edges[horz_edge].top.x
                    && let Some(next) = self.edges[horz_edge].next_in_lml
                    && self.edges[current].dx < self.edges[next].dx
                {
                    break;
                }

                if self.edges[horz_edge].out_index >= 0 {
                    let new_op = self.add_out_pt(horz_edge, self.edges[current].curr)?;
                    op1 = Some(new_op);
                    self.join_overlapping_horizontals(horz_edge, new_op)?;
                    self.add_ghost_join(new_op, self.edges[horz_edge].bot);
                }

                if e_max_pair == Some(current) && is_last_horz {
                    if self.edges[horz_edge].out_index >= 0 {
                        self.add_local_max_poly(horz_edge, current, self.edges[horz_edge].top)?;
                    }
                    self.delete_from_ael(horz_edge);
                    self.delete_from_ael(current);
                    return Ok(());
                }

                let point = IntPoint::new(self.edges[current].curr.x, self.edges[horz_edge].curr.y);
                if direction == Direction::LeftToRight {
                    self.intersect_edges(horz_edge, current, point)?;
                } else {
                    self.intersect_edges(current, horz_edge, point)?;
                }
                let e_next = self.get_next_in_ael(current, direction);
                self.swap_positions_in_ael(horz_edge, current);
                e = e_next;
            }

            let Some(next) = self.edges[horz_edge].next_in_lml else {
                break;
            };
            if !self.edges[next].is_horizontal() {
                break;
            }
            self.update_edge_into_ael(&mut horz_edge)?;
            if self.edges[horz_edge].out_index >= 0 {
                self.add_out_pt(horz_edge, self.edges[horz_edge].bot)?;
            }
            (direction, horz_left, horz_right) = self.get_horz_direction(horz_edge);
        }

        if self.edges[horz_edge].out_index >= 0 && op1.is_none() {
            let last = self.get_last_out_pt(horz_edge)?;
            self.join_overlapping_horizontals(horz_edge, last)?;
            self.add_ghost_join(last, self.edges[horz_edge].top);
        }

        if self.edges[horz_edge].next_in_lml.is_none() {
            if self.edges[horz_edge].out_index >= 0 {
                self.add_out_pt(horz_edge, self.edges[horz_edge].top)?;
            }
            self.delete_from_ael(horz_edge);
            return Ok(());
        }
        if self.edges[horz_edge].out_index < 0 {
            return self.update_edge_into_ael(&mut horz_edge);
        }

        let op1 = self.add_out_pt(horz_edge, self.edges[horz_edge].top)?;
        self.update_edge_into_ael(&mut horz_edge)?;
        // The edge isn't horizontal any more.
        let horz = self.edges[horz_edge];
        let full_range = self.use_full_range;
        let touches = |other: &Edge| {
            other.curr.x == horz.bot.x
                && other.curr.y == horz.bot.y
                && other.out_index >= 0
                && other.curr.y > other.top.y
                && slopes_equal_edges(&horz, other, full_range)
        };
        if let Some(prev) = horz.prev_in_ael
            && touches(&self.edges[prev])
        {
            let op2 = self.add_out_pt(prev, horz.bot)?;
            self.add_join(Some(op1), Some(op2), horz.top);
        } else if let Some(next) = horz.next_in_ael
            && touches(&self.edges[next])
        {
            let op2 = self.add_out_pt(next, horz.bot)?;
            self.add_join(Some(op1), Some(op2), horz.top);
        }
        Ok(())
    }

    /// `ProcessIntersections`: the crossings below `top_y`, made between
    /// adjacent edges only. False when they can't be ordered so.
    fn process_intersections(&mut self, top_y: i64) -> Outcome<bool> {
        if self.active_edges.is_none() {
            return Ok(true);
        }
        // A clipper reused after a failed clip still holds that clip's
        // active edges (when nothing reset them) and intersections.
        if self.minima.is_empty() || !self.intersections.is_empty() {
            return Err(Failure::undefined("Clipper reused after a failed clip reads freed edges"));
        }
        self.build_intersect_list(top_y);
        let count = self.intersections.len();
        if count == 0 {
            return Ok(true);
        }
        if count != 1 && !self.fixup_intersection_order() {
            return Ok(false);
        }
        self.process_intersect_list()?;
        self.sorted_edges = None;
        Ok(true)
    }

    /// `BuildIntersectList`: a bubble sort of the edges by X at `top_y`,
    /// each swap a crossing.
    fn build_intersect_list(&mut self, top_y: i64) {
        let Some(first) = self.active_edges else {
            return;
        };
        self.sorted_edges = Some(first);
        let mut e = Some(first);
        while let Some(index) = e {
            let edge = &mut self.edges[index];
            edge.prev_in_sel = edge.prev_in_ael;
            edge.next_in_sel = edge.next_in_ael;
            edge.curr.x = top_x(edge, top_y);
            e = edge.next_in_ael;
        }

        loop {
            let mut is_modified = false;
            let Some(mut e) = self.sorted_edges else {
                break;
            };
            while let Some(e_next) = self.edges[e].next_in_sel {
                if self.edges[e].curr.x > self.edges[e_next].curr.x {
                    let mut point = intersect_point(&self.edges[e], &self.edges[e_next]);
                    if point.y < top_y {
                        point = IntPoint::new(top_x(&self.edges[e], top_y), top_y);
                    }
                    let node = IntersectNode { edge_1: e, edge_2: e_next, point };
                    self.intersections.push(node);
                    self.swap_positions_in_sel(e, e_next);
                    is_modified = true;
                } else {
                    e = e_next;
                }
            }
            let Some(prev) = self.edges[e].prev_in_sel else {
                break;
            };
            self.edges[prev].next_in_sel = None;
            if !is_modified {
                break;
            }
        }
        self.sorted_edges = None;
    }

    /// `ProcessIntersectList`.
    fn process_intersect_list(&mut self) -> Outcome<()> {
        for index in 0..self.intersections.len() {
            let node = self.intersections[index];
            self.intersect_edges(node.edge_1, node.edge_2, node.point)?;
            self.swap_positions_in_ael(node.edge_1, node.edge_2);
        }
        self.intersections.clear();
        Ok(())
    }

    fn edges_adjacent(&self, node: &IntersectNode) -> bool {
        let edge_1 = &self.edges[node.edge_1];
        edge_1.next_in_sel == Some(node.edge_2) || edge_1.prev_in_sel == Some(node.edge_2)
    }

    /// `FixupIntersectionOrder`: the crossings sorted bottom first, then
    /// reordered so each is between edges adjacent at that point.
    fn fixup_intersection_order(&mut self) -> bool {
        self.copy_ael_to_sel();
        msvc::sort(&mut self.intersections, |a, b| b.point.y < a.point.y);
        let count = self.intersections.len();
        for i in 0..count {
            if !self.edges_adjacent(&self.intersections[i]) {
                let mut j = i + 1;
                while j < count && !self.edges_adjacent(&self.intersections[j]) {
                    j += 1;
                }
                if j == count {
                    return false;
                }
                self.intersections.swap(i, j);
            }
            let node = self.intersections[i];
            self.swap_positions_in_sel(node.edge_1, node.edge_2);
        }
        true
    }

    /// `DoMaxima`: an edge ending at a local maximum, crossing the edges
    /// between it and its pair.
    fn do_maxima(&mut self, e: usize) -> Outcome<()> {
        let Some(e_max_pair) = self.get_maxima_pair_ex(e) else {
            if self.edges[e].out_index >= 0 {
                self.add_out_pt(e, self.edges[e].top)?;
            }
            self.delete_from_ael(e);
            return Ok(());
        };

        let mut e_next = self.edges[e].next_in_ael;
        while let Some(next) = e_next
            && next != e_max_pair
        {
            self.intersect_edges(e, next, self.edges[e].top)?;
            self.swap_positions_in_ael(e, next);
            e_next = self.edges[e].next_in_ael;
        }

        let out_index = self.edges[e].out_index;
        let pair_out_index = self.edges[e_max_pair].out_index;
        if out_index == UNASSIGNED && pair_out_index == UNASSIGNED {
            self.delete_from_ael(e);
            self.delete_from_ael(e_max_pair);
        } else if out_index >= 0 && pair_out_index >= 0 {
            self.add_local_max_poly(e, e_max_pair, self.edges[e].top)?;
            self.delete_from_ael(e);
            self.delete_from_ael(e_max_pair);
        } else {
            return Err(Failure::Std("DoMaxima error".to_string()));
        }
        Ok(())
    }

    /// `ProcessEdgesAtTopOfScanbeam`: maxima closed, edges moved to
    /// `top_y`, horizontals processed, intermediate vertices promoted.
    fn process_edges_at_top_of_scanbeam(&mut self, top_y: i64) -> Outcome<()> {
        let mut e = self.active_edges;
        while let Some(mut current) = e {
            let edge = self.edges[current];
            let mut is_maxima_edge = edge.top.y == top_y && edge.next_in_lml.is_none();
            if is_maxima_edge && let Some(pair) = self.get_maxima_pair_ex(current) {
                is_maxima_edge = !self.edges[pair].is_horizontal();
            }

            if is_maxima_edge {
                let e_prev = edge.prev_in_ael;
                self.do_maxima(current)?;
                e = match e_prev {
                    None => self.active_edges,
                    Some(prev) => self.edges[prev].next_in_ael,
                };
                continue;
            }

            let promotes_horizontal =
                edge.top.y == top_y && edge.next_in_lml.is_some_and(|next| self.edges[next].is_horizontal());
            if promotes_horizontal {
                self.update_edge_into_ael(&mut current)?;
                if self.edges[current].out_index >= 0 {
                    self.add_out_pt(current, self.edges[current].bot)?;
                }
                self.add_edge_to_sel(current);
            } else {
                let x = top_x(&self.edges[current], top_y);
                self.edges[current].curr = IntPoint::new(x, top_y);
            }
            e = self.edges[current].next_in_ael;
        }

        self.process_horizontals()?;

        let mut e = self.active_edges;
        while let Some(mut current) = e {
            let edge = self.edges[current];
            if edge.top.y == top_y && edge.next_in_lml.is_some() {
                let mut op = None;
                if edge.out_index >= 0 {
                    op = Some(self.add_out_pt(current, edge.top)?);
                }
                self.update_edge_into_ael(&mut current)?;

                // Output polygons sharing an edge get joined later.
                let promoted = self.edges[current];
                let full_range = self.use_full_range;
                let touches = |other: &Edge| {
                    other.curr.x == promoted.bot.x
                        && other.curr.y == promoted.bot.y
                        && op.is_some()
                        && other.out_index >= 0
                        && other.curr.y > other.top.y
                        && slopes_equal_4(promoted.curr, promoted.top, other.curr, other.top, full_range)
                };
                if let Some(prev) = promoted.prev_in_ael
                    && touches(&self.edges[prev])
                {
                    let op2 = self.add_out_pt(prev, promoted.bot)?;
                    self.add_join(op, Some(op2), promoted.top);
                } else if let Some(next) = promoted.next_in_ael
                    && touches(&self.edges[next])
                {
                    let op2 = self.add_out_pt(next, promoted.bot)?;
                    self.add_join(op, Some(op2), promoted.top);
                }
            }
            e = self.edges[current].next_in_ael;
        }
        Ok(())
    }

    /// `FixupOutPolygon`: duplicate points and the middle of collinear
    /// runs removed; a polygon left with fewer than 3 points goes.
    fn fixup_out_polygon(&mut self, out_rec: usize) {
        let mut last_ok: Option<usize> = None;
        self.out_recs[out_rec].bottom_point = None;
        let Some(mut pp) = self.out_recs[out_rec].points else {
            return;
        };
        loop {
            let OutPt { point, next, prev, .. } = self.out_pts[pp];
            if prev == pp || prev == next {
                self.out_recs[out_rec].points = None;
                return;
            }
            let prev_point = self.out_pts[prev].point;
            let next_point = self.out_pts[next].point;
            if point == next_point || point == prev_point || slopes_equal_3(prev_point, point, next_point, self.use_full_range) {
                last_ok = None;
                self.out_pts[prev].next = next;
                self.out_pts[next].prev = prev;
                pp = prev;
            } else if last_ok == Some(pp) {
                break;
            } else {
                if last_ok.is_none() {
                    last_ok = Some(pp);
                }
                pp = next;
            }
        }
        self.out_recs[out_rec].points = Some(pp);
    }

    fn point_count(&self, start: usize) -> usize {
        let mut count = 0;
        let mut p = start;
        loop {
            count += 1;
            p = self.out_pts[p].next;
            if p == start {
                return count;
            }
        }
    }

    /// `BuildResult`: each polygon of 2 or more points, walked backwards
    /// from its last point.
    fn build_result(&self) -> Vec<Path> {
        let mut polygons = Vec::with_capacity(self.out_recs.len());
        for out_rec in &self.out_recs {
            let Some(points) = out_rec.points else {
                continue;
            };
            let mut p = self.out_pts[points].prev;
            let count = self.point_count(p);
            if count < 2 {
                continue;
            }
            let mut path = Vec::with_capacity(count);
            for _ in 0..count {
                path.push(self.out_pts[p].point);
                p = self.out_pts[p].prev;
            }
            polygons.push(path);
        }
        polygons
    }

    /// `Area` of an output polygon (coordinate sums wrap before the
    /// conversion).
    fn area_of(&self, start: usize) -> f64 {
        let mut total = 0.0;
        let mut op = start;
        loop {
            let OutPt { point, next, prev, .. } = self.out_pts[op];
            let prev_point = self.out_pts[prev].point;
            let sum = prev_point.x.wrapping_add(point.x) as f64;
            let difference = prev_point.y.wrapping_sub(point.y) as f64;
            total += sum * difference;
            op = next;
            if op == start {
                return total * 0.5;
            }
        }
    }

    /// `ReversePolyPtLinks`.
    fn reverse_poly_pt_links(&mut self, start: usize) {
        let mut pp1 = start;
        loop {
            let pp2 = self.out_pts[pp1].next;
            self.out_pts[pp1].next = self.out_pts[pp1].prev;
            self.out_pts[pp1].prev = pp2;
            pp1 = pp2;
            if pp1 == start {
                return;
            }
        }
    }

    /// `GetBottomPt`: the lowest, leftmost point (of several there, the
    /// one `FirstIsBottomPt` prefers).
    fn get_bottom_pt(&self, mut pp: usize) -> usize {
        let mut dups: Option<usize> = None;
        let mut p = self.out_pts[pp].next;
        while p != pp {
            let point = self.out_pts[p].point;
            let bottom = self.out_pts[pp].point;
            if point.y > bottom.y {
                pp = p;
                dups = None;
            } else if point.y == bottom.y && point.x <= bottom.x {
                if point.x < bottom.x {
                    dups = None;
                    pp = p;
                } else if self.out_pts[p].next != pp && self.out_pts[p].prev != pp {
                    dups = Some(p);
                }
            }
            p = self.out_pts[p].next;
        }
        if let Some(mut dups) = dups {
            // At least two vertices at the bottom point.
            while dups != p {
                if !self.first_is_bottom_pt(p, dups) {
                    pp = dups;
                }
                dups = self.out_pts[dups].next;
                while self.out_pts[dups].point != self.out_pts[pp].point {
                    dups = self.out_pts[dups].next;
                }
            }
        }
        pp
    }

    /// The slope magnitudes either side of a bottom point.
    fn bottom_slopes(&self, bottom: usize) -> (f64, f64) {
        let point = self.out_pts[bottom].point;
        let mut p = self.out_pts[bottom].prev;
        while self.out_pts[p].point == point && p != bottom {
            p = self.out_pts[p].prev;
        }
        let dx_prev = get_dx(point, self.out_pts[p].point).abs();
        p = self.out_pts[bottom].next;
        while self.out_pts[p].point == point && p != bottom {
            p = self.out_pts[p].next;
        }
        let dx_next = get_dx(point, self.out_pts[p].point).abs();
        (dx_prev, dx_next)
    }

    /// `FirstIsBottomPt`.
    fn first_is_bottom_pt(&self, bottom_1: usize, bottom_2: usize) -> bool {
        let (dx1p, dx1n) = self.bottom_slopes(bottom_1);
        let (dx2p, dx2n) = self.bottom_slopes(bottom_2);
        if std_max(dx1p, dx1n) == std_max(dx2p, dx2n) && std_min(dx1p, dx1n) == std_min(dx2p, dx2n) {
            return self.area_of(bottom_1) > 0.0;
        }
        (dx1p >= dx2p && dx1p >= dx2n) || (dx1n >= dx2p && dx1n >= dx2n)
    }

    /// `PointInPolygon` against an output ring: 0 outside, 1 inside, -1
    /// on its boundary.
    fn point_in_polygon(&self, point: IntPoint, start: usize) -> i32 {
        let mut result = 0;
        let mut op = start;
        loop {
            let here = self.out_pts[op].point;
            let next_op = self.out_pts[op].next;
            let next = self.out_pts[next_op].point;
            if next.y == point.y && (next.x == point.x || (here.y == point.y && (next.x > point.x) == (here.x < point.x))) {
                return -1;
            }
            if (here.y < point.y) != (next.y < point.y) {
                let crossing = |result: &mut i32| -> bool {
                    let d = here.x.wrapping_sub(point.x) as f64 * next.y.wrapping_sub(point.y) as f64
                        - next.x.wrapping_sub(point.x) as f64 * here.y.wrapping_sub(point.y) as f64;
                    if d == 0.0 {
                        return true;
                    }
                    if (d > 0.0) == (next.y > here.y) {
                        *result = 1 - *result;
                    }
                    false
                };
                if here.x >= point.x {
                    if next.x > point.x {
                        result = 1 - result;
                    } else if crossing(&mut result) {
                        return -1;
                    }
                } else if next.x > point.x && crossing(&mut result) {
                    return -1;
                }
            }
            op = next_op;
            if op == start {
                return result;
            }
        }
    }

    /// `Poly2ContainsPoly1`: by the first point of ring 1 not on ring 2.
    fn poly2_contains_poly1(&self, out_pt_1: usize, out_pt_2: usize) -> bool {
        let mut op = out_pt_1;
        loop {
            let result = self.point_in_polygon(self.out_pts[op].point, out_pt_2);
            if result >= 0 {
                return result > 0;
            }
            op = self.out_pts[op].next;
            if op == out_pt_1 {
                return true;
            }
        }
    }

    /// `DupOutPt`: a copy linked in after (or before) the point.
    fn dup_out_pt(&mut self, out_pt: usize, insert_after: bool) -> usize {
        let source = self.out_pts[out_pt];
        let result = self.out_pts.len();
        if insert_after {
            let copy = OutPt { index: source.index, point: source.point, next: source.next, prev: out_pt };
            self.out_pts.push(copy);
            self.out_pts[source.next].prev = result;
            self.out_pts[out_pt].next = result;
        } else {
            let copy = OutPt { index: source.index, point: source.point, next: out_pt, prev: source.prev };
            self.out_pts.push(copy);
            self.out_pts[source.prev].next = result;
            self.out_pts[out_pt].prev = result;
        }
        result
    }

    /// One ring's part of `JoinHorz`: the point to join at and its copy.
    fn join_horz_end(&mut self, mut op: usize, direction: Direction, point: IntPoint, discard_left: bool) -> (usize, usize) {
        let insert_after = if direction == Direction::LeftToRight { !discard_left } else { discard_left };
        if direction == Direction::LeftToRight {
            loop {
                let here = self.out_pts[op].point;
                let next = self.out_pts[self.out_pts[op].next].point;
                if !(next.x <= point.x && next.x >= here.x && next.y == point.y) {
                    break;
                }
                op = self.out_pts[op].next;
            }
            if discard_left && self.out_pts[op].point.x != point.x {
                op = self.out_pts[op].next;
            }
        } else {
            loop {
                let here = self.out_pts[op].point;
                let next = self.out_pts[self.out_pts[op].next].point;
                if !(next.x >= point.x && next.x <= here.x && next.y == point.y) {
                    break;
                }
                op = self.out_pts[op].next;
            }
            if !discard_left && self.out_pts[op].point.x != point.x {
                op = self.out_pts[op].next;
            }
        }
        let mut op_b = self.dup_out_pt(op, insert_after);
        if self.out_pts[op_b].point != point {
            op = op_b;
            self.out_pts[op].point = point;
            op_b = self.dup_out_pt(op, insert_after);
        }
        (op, op_b)
    }

    /// `JoinHorz`: two overlapping horizontal runs joined at `point`.
    fn join_horz(&mut self, op1: usize, op1b: usize, op2: usize, op2b: usize, point: IntPoint, discard_left: bool) -> bool {
        let direction_of = |a: IntPoint, b: IntPoint| if a.x > b.x { Direction::RightToLeft } else { Direction::LeftToRight };
        let direction_1 = direction_of(self.out_pts[op1].point, self.out_pts[op1b].point);
        let direction_2 = direction_of(self.out_pts[op2].point, self.out_pts[op2b].point);
        if direction_1 == direction_2 {
            return false;
        }
        let (op1, op1b) = self.join_horz_end(op1, direction_1, point, discard_left);
        let (op2, op2b) = self.join_horz_end(op2, direction_2, point, discard_left);
        if (direction_1 == Direction::LeftToRight) == discard_left {
            self.out_pts[op1].prev = op2;
            self.out_pts[op2].next = op1;
            self.out_pts[op1b].next = op2b;
            self.out_pts[op2b].prev = op1b;
        } else {
            self.out_pts[op1].next = op2;
            self.out_pts[op2].prev = op1;
            self.out_pts[op1b].prev = op2b;
            self.out_pts[op2b].next = op1b;
        }
        true
    }

    /// The joined rings relinked at `op1`/`op2` (reversed or not), the
    /// join's points now `op1` and its copy.
    fn link_join(&mut self, join_index: usize, op1: usize, op2: usize, reverse: bool) {
        let op1b;
        if reverse {
            op1b = self.dup_out_pt(op1, false);
            let op2b = self.dup_out_pt(op2, true);
            self.out_pts[op1].prev = op2;
            self.out_pts[op2].next = op1;
            self.out_pts[op1b].next = op2b;
            self.out_pts[op2b].prev = op1b;
        } else {
            op1b = self.dup_out_pt(op1, true);
            let op2b = self.dup_out_pt(op2, false);
            self.out_pts[op1].next = op2;
            self.out_pts[op2].prev = op1;
            self.out_pts[op1b].prev = op2b;
            self.out_pts[op2b].next = op1b;
        }
        self.joins[join_index].out_pt_1 = Some(op1);
        self.joins[join_index].out_pt_2 = Some(op1b);
    }

    /// The neighbour of `op` along a ring (forwards or back) at another
    /// point, and whether the ring runs down or off the join's line there.
    fn join_neighbour(&self, op: usize, forwards: bool, offset_point: IntPoint) -> (usize, bool) {
        let point = self.out_pts[op].point;
        let step = |p: usize| if forwards { self.out_pts[p].next } else { self.out_pts[p].prev };
        let mut op_b = step(op);
        while self.out_pts[op_b].point == point && op_b != op {
            op_b = step(op_b);
        }
        let other = self.out_pts[op_b].point;
        let wrong_way = other.y > point.y || !slopes_equal_3(point, other, offset_point, self.use_full_range);
        (op_b, wrong_way)
    }

    /// `JoinPoints`: one join made, when the rings allow it.
    fn join_points(&mut self, join_index: usize, out_rec_1: usize, out_rec_2: usize) -> Outcome<bool> {
        let join = self.joins[join_index];
        let mut op1 = self.out_point(join.out_pt_1)?;
        let mut op2 = self.out_point(join.out_pt_2)?;
        let offset_point = join.offset_point;
        let is_horizontal = self.out_pts[op1].point.y == offset_point.y;

        if is_horizontal && offset_point == self.out_pts[op1].point && offset_point == self.out_pts[op2].point {
            // A strictly simple join.
            if out_rec_1 != out_rec_2 {
                return Ok(false);
            }
            let mut op1b = self.out_pts[op1].next;
            while op1b != op1 && self.out_pts[op1b].point == offset_point {
                op1b = self.out_pts[op1b].next;
            }
            let reverse_1 = self.out_pts[op1b].point.y > offset_point.y;
            let mut op2b = self.out_pts[op2].next;
            while op2b != op2 && self.out_pts[op2b].point == offset_point {
                op2b = self.out_pts[op2b].next;
            }
            let reverse_2 = self.out_pts[op2b].point.y > offset_point.y;
            if reverse_1 == reverse_2 {
                return Ok(false);
            }
            self.link_join(join_index, op1, op2, reverse_1);
            return Ok(true);
        }

        if is_horizontal {
            // Where the horizontal runs overlap isn't known yet.
            let same_y = |a: usize, b: usize| self.out_pts[a].point.y == self.out_pts[b].point.y;
            let mut op1b = op1;
            while same_y(self.out_pts[op1].prev, op1) && self.out_pts[op1].prev != op1b && self.out_pts[op1].prev != op2 {
                op1 = self.out_pts[op1].prev;
            }
            while same_y(self.out_pts[op1b].next, op1b) && self.out_pts[op1b].next != op1 && self.out_pts[op1b].next != op2 {
                op1b = self.out_pts[op1b].next;
            }
            if self.out_pts[op1b].next == op1 || self.out_pts[op1b].next == op2 {
                // A flat polygon.
                return Ok(false);
            }
            let mut op2b = op2;
            while same_y(self.out_pts[op2].prev, op2) && self.out_pts[op2].prev != op2b && self.out_pts[op2].prev != op1b {
                op2 = self.out_pts[op2].prev;
            }
            while same_y(self.out_pts[op2b].next, op2b) && self.out_pts[op2b].next != op2 && self.out_pts[op2b].next != op1 {
                op2b = self.out_pts[op2b].next;
            }
            if self.out_pts[op2b].next == op2 || self.out_pts[op2b].next == op1 {
                return Ok(false);
            }

            let p1 = self.out_pts[op1].point;
            let p1b = self.out_pts[op1b].point;
            let p2 = self.out_pts[op2].point;
            let p2b = self.out_pts[op2b].point;
            let Some((left, right)) = get_overlap(p1.x, p1b.x, p2.x, p2b.x) else {
                return Ok(false);
            };
            // The spike the join leaves is cleaned up later; neither op1
            // nor op2 may be on the discarded side.
            let (point, discard_left_side) = if p1.x >= left && p1.x <= right {
                (p1, p1.x > p1b.x)
            } else if p2.x >= left && p2.x <= right {
                (p2, p2.x > p2b.x)
            } else if p1b.x >= left && p1b.x <= right {
                (p1b, p1b.x > p1.x)
            } else {
                (p2b, p2b.x > p2.x)
            };
            self.joins[join_index].out_pt_1 = Some(op1);
            self.joins[join_index].out_pt_2 = Some(op2);
            return Ok(self.join_horz(op1, op1b, op2, op2b, point, discard_left_side));
        }

        // A non-horizontal join: the rings must both run up along it.
        let (mut op1b, reverse_1) = self.join_neighbour(op1, true, offset_point);
        if reverse_1 {
            let (back, wrong_way) = self.join_neighbour(op1, false, offset_point);
            if wrong_way {
                return Ok(false);
            }
            op1b = back;
        }
        let (mut op2b, reverse_2) = self.join_neighbour(op2, true, offset_point);
        if reverse_2 {
            let (back, wrong_way) = self.join_neighbour(op2, false, offset_point);
            if wrong_way {
                return Ok(false);
            }
            op2b = back;
        }
        if op1b == op1 || op2b == op2 || op1b == op2b || (out_rec_1 == out_rec_2 && reverse_1 == reverse_2) {
            return Ok(false);
        }
        self.link_join(join_index, op1, op2, reverse_1);
        Ok(true)
    }

    /// `UpdateOutPtIdxs`.
    fn update_out_pt_idxs(&mut self, out_rec: usize, start: usize) {
        let index = self.out_recs[out_rec].index;
        let mut op = start;
        loop {
            self.out_pts[op].index = index;
            op = self.out_pts[op].prev;
            if op == start {
                return;
            }
        }
    }

    /// `JoinCommonEdges`: the joins made, splitting a polygon in two or
    /// merging two into one.
    fn join_common_edges(&mut self) -> Outcome<()> {
        for index in 0..self.joins.len() {
            let join = self.joins[index];
            let out_pt_1 = self.out_point(join.out_pt_1)?;
            let out_pt_2 = self.out_point(join.out_pt_2)?;
            let out_rec_1 = self.get_out_rec(self.out_pts[out_pt_1].index)?;
            let mut out_rec_2 = self.get_out_rec(self.out_pts[out_pt_2].index)?;
            if self.out_recs[out_rec_1].points.is_none() || self.out_recs[out_rec_2].points.is_none() {
                continue;
            }

            // The fragment with the right hole state, before the join.
            let hole_state_rec = if out_rec_1 == out_rec_2 {
                out_rec_1
            } else if self.out_rec_1_right_of_out_rec_2(out_rec_1, out_rec_2)? {
                out_rec_2
            } else if self.out_rec_1_right_of_out_rec_2(out_rec_2, out_rec_1)? {
                out_rec_1
            } else {
                self.get_lowermost_rec(out_rec_1, out_rec_2)?
            };

            if !self.join_points(index, out_rec_1, out_rec_2)? {
                continue;
            }

            if out_rec_1 != out_rec_2 {
                // Two polygons joined into one.
                self.out_recs[out_rec_2].points = None;
                self.out_recs[out_rec_2].bottom_point = None;
                self.out_recs[out_rec_2].index = self.out_recs[out_rec_1].index;
                self.out_recs[out_rec_1].is_hole = self.out_recs[hole_state_rec].is_hole;
                if hole_state_rec == out_rec_2 {
                    self.out_recs[out_rec_1].first_left = self.out_recs[out_rec_2].first_left;
                }
                self.out_recs[out_rec_2].first_left = Some(out_rec_1);
                continue;
            }

            // One polygon split into two.
            let join = self.joins[index];
            let points_1 = self.out_point(join.out_pt_1)?;
            let points_2 = self.out_point(join.out_pt_2)?;
            self.out_recs[out_rec_1].points = Some(points_1);
            self.out_recs[out_rec_1].bottom_point = None;
            out_rec_2 = self.out_recs.len();
            let record = OutRec { index: out_rec_2 as i32, is_hole: false, first_left: None, points: Some(points_2), bottom_point: None };
            self.out_recs.push(record);
            self.update_out_pt_idxs(out_rec_2, points_2);

            if self.poly2_contains_poly1(points_2, points_1) {
                // The first contains the second.
                self.out_recs[out_rec_2].is_hole = !self.out_recs[out_rec_1].is_hole;
                self.out_recs[out_rec_2].first_left = Some(out_rec_1);
                if self.out_recs[out_rec_2].is_hole == (self.area_of(points_2) > 0.0) {
                    self.reverse_poly_pt_links(points_2);
                }
            } else if self.poly2_contains_poly1(points_1, points_2) {
                // The second contains the first.
                self.out_recs[out_rec_2].is_hole = self.out_recs[out_rec_1].is_hole;
                self.out_recs[out_rec_1].is_hole = !self.out_recs[out_rec_2].is_hole;
                self.out_recs[out_rec_2].first_left = self.out_recs[out_rec_1].first_left;
                self.out_recs[out_rec_1].first_left = Some(out_rec_2);
                if self.out_recs[out_rec_1].is_hole == (self.area_of(points_1) > 0.0) {
                    self.reverse_poly_pt_links(points_1);
                }
            } else {
                // Separate polygons.
                self.out_recs[out_rec_2].is_hole = self.out_recs[out_rec_1].is_hole;
                self.out_recs[out_rec_2].first_left = self.out_recs[out_rec_1].first_left;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: i64, y: i64, size: i64) -> Path {
        vec![IntPoint::new(x, y), IntPoint::new(x + size, y), IntPoint::new(x + size, y + size), IntPoint::new(x, y + size)]
    }

    #[test]
    fn overlapping_squares_unite_into_one_outline() {
        let mut clipper = Clipper::new();
        clipper.add_path(&square(0, 0, 10), PolyType::Subject).unwrap();
        clipper.add_path(&square(5, 5, 10), PolyType::Subject).unwrap();
        let union = clipper.execute(ClipType::Union).unwrap();
        assert_eq!(union.len(), 1);
        assert_eq!(union[0].len(), 8);
        assert_eq!(area(&union[0]).abs(), 175.0);
    }

    #[test]
    fn a_hole_cut_from_a_square_leaves_two_rings() {
        let mut clipper = Clipper::new();
        clipper.add_path(&square(0, 0, 10), PolyType::Subject).unwrap();
        clipper.add_path(&square(3, 3, 4), PolyType::Clip).unwrap();
        let difference = clipper.execute(ClipType::Difference).unwrap();
        assert_eq!(difference.len(), 2);
        let areas: Vec<f64> = difference.iter().map(|path| area(path)).collect();
        assert!(areas.contains(&100.0) && areas.contains(&-16.0), "{areas:?}");
    }

    #[test]
    fn coordinates_past_the_range_throw() {
        let mut clipper = Clipper::new();
        let inside = square(0, 0, HIGH_RANGE);
        assert!(clipper.add_path(&inside, PolyType::Subject).unwrap());
        let outside = square(0, 0, HIGH_RANGE + 1);
        assert!(matches!(clipper.add_path(&outside, PolyType::Subject), Err(Failure::Std(_))));
    }

    #[test]
    fn products_compare_like_clipper_int128() {
        assert_eq!(int128_mul(-3, 4), int128_mul(4, -3));
        assert_eq!(int128_mul(1 << 40, 1 << 40), (1 << 16, 0));
        assert_eq!(int128_mul(-(1 << 40), 1 << 40), (-(1 << 16), 0));
    }
}
