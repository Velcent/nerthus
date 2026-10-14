//! poly2tri (the 2018+ version Assimp bundles, which throws
//! `runtime_error` in some places upstream asserts): constrained Delaunay
//! triangulation of a polygon with holes by a sweep line. Points are
//! identities (two equal points are different vertices). The remaining
//! asserts are live in Gaea's build (a corner that isn't one ends the
//! process); reads through a missing triangle are undefined behaviour, an
//! edge index of -1 writes the flag before the array.

use super::{Failure, Outcome};
use crate::msvc;

/// `kAlpha`: the margin of the initial triangle's head and tail.
const ALPHA: f64 = 0.3;
const EPSILON: f64 = 1e-12;
const PI_3DIV4: f64 = 3.0 * std::f64::consts::PI / 4.0;
const PI_DIV2: f64 = std::f64::consts::FRAC_PI_2;
/// Recursion past this many nested calls stands for the native stack
/// running out (recursion that never ends gets here too).
const MAX_DEPTH: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Point {
    pub(super) x: f64,
    pub(super) y: f64,
}

impl Point {
    pub(super) fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A `p2t::Point`: its place and the edges whose upper end it is.
struct Vertex {
    point: Point,
    edge_list: Vec<usize>,
}

/// `p2t::Edge`: `p` is the lower end (by y, then x), `q` the upper.
#[derive(Clone, Copy)]
struct Edge {
    p: usize,
    q: usize,
}

#[derive(Clone, Copy)]
struct Triangle {
    constrained_edge: [bool; 3],
    delaunay_edge: [bool; 3],
    points: [usize; 3],
    neighbors: [Option<usize>; 3],
    interior: bool,
}

impl Triangle {
    fn new(a: usize, b: usize, c: usize) -> Self {
        Self { constrained_edge: [false; 3], delaunay_edge: [false; 3], points: [a, b, c], neighbors: [None; 3], interior: false }
    }

    fn contains(&self, p: usize) -> bool {
        self.points.contains(&p)
    }

    fn contains_both(&self, p: usize, q: usize) -> bool {
        self.contains(p) && self.contains(q)
    }

    /// `MarkNeighbor(p1, p2, t)`: `t` across the edge p1-p2.
    fn mark_neighbor_across(&mut self, p1: usize, p2: usize, t: usize) -> Outcome<()> {
        let [a, b, c] = self.points;
        if (p1 == c && p2 == b) || (p1 == b && p2 == c) {
            self.neighbors[0] = Some(t);
        } else if (p1 == a && p2 == c) || (p1 == c && p2 == a) {
            self.neighbors[1] = Some(t);
        } else if (p1 == a && p2 == b) || (p1 == b && p2 == a) {
            self.neighbors[2] = Some(t);
        } else {
            return not_a_corner();
        }
        Ok(())
    }

    fn index(&self, p: usize) -> Outcome<usize> {
        match self.points.iter().position(|&point| point == p) {
            Some(index) => Ok(index),
            None => not_a_corner(),
        }
    }

    /// `EdgeIndex`: the index of the edge p1-p2 (opposite its third point).
    fn edge_index(&self, p1: usize, p2: usize) -> Option<usize> {
        let [a, b, c] = self.points;
        if a == p1 {
            if b == p2 {
                return Some(2);
            } else if c == p2 {
                return Some(1);
            }
        } else if b == p1 {
            if c == p2 {
                return Some(0);
            } else if a == p2 {
                return Some(2);
            }
        } else if c == p1 {
            if a == p2 {
                return Some(1);
            } else if b == p2 {
                return Some(0);
            }
        }
        None
    }

    /// `MarkConstrainedEdge(p, q)`.
    fn mark_constrained_edge(&mut self, p: usize, q: usize) {
        let [a, b, c] = self.points;
        if (q == a && p == b) || (q == b && p == a) {
            self.constrained_edge[2] = true;
        } else if (q == a && p == c) || (q == c && p == a) {
            self.constrained_edge[1] = true;
        } else if (q == b && p == c) || (q == c && p == b) {
            self.constrained_edge[0] = true;
        }
    }

    /// `delaunay_edge[index] = value` for an `EdgeIndex` result: -1 is the
    /// byte before the array, the last constrained flag.
    fn set_delaunay_edge_at(&mut self, index: Option<usize>, value: bool) {
        match index {
            Some(index) => self.delaunay_edge[index] = value,
            None => self.constrained_edge[2] = value,
        }
    }

    fn point_cw(&self, p: usize) -> Outcome<usize> {
        let [a, b, c] = self.points;
        if p == a {
            Ok(c)
        } else if p == b {
            Ok(a)
        } else if p == c {
            Ok(b)
        } else {
            not_a_corner()
        }
    }

    fn point_ccw(&self, p: usize) -> Outcome<usize> {
        let [a, b, c] = self.points;
        if p == a {
            Ok(b)
        } else if p == b {
            Ok(c)
        } else if p == c {
            Ok(a)
        } else {
            not_a_corner()
        }
    }

    /// Which slot `NeighborAcross` and the CW/CCW accessors use for `p`:
    /// 0, 1 or (for any other point) 2.
    fn slot(&self, p: usize) -> usize {
        if p == self.points[0] {
            0
        } else if p == self.points[1] {
            1
        } else {
            2
        }
    }

    fn neighbor_across(&self, p: usize) -> Option<usize> {
        self.neighbors[self.slot(p)]
    }

    fn neighbor_cw(&self, p: usize) -> Option<usize> {
        self.neighbors[(self.slot(p) + 1) % 3]
    }

    fn neighbor_ccw(&self, p: usize) -> Option<usize> {
        self.neighbors[(self.slot(p) + 2) % 3]
    }

    fn constrained_edge_cw(&self, p: usize) -> bool {
        self.constrained_edge[(self.slot(p) + 1) % 3]
    }

    fn constrained_edge_ccw(&self, p: usize) -> bool {
        self.constrained_edge[(self.slot(p) + 2) % 3]
    }

    fn set_constrained_edge_cw(&mut self, p: usize, value: bool) {
        self.constrained_edge[(self.slot(p) + 1) % 3] = value;
    }

    fn set_constrained_edge_ccw(&mut self, p: usize, value: bool) {
        self.constrained_edge[(self.slot(p) + 2) % 3] = value;
    }

    fn delaunay_edge_cw(&self, p: usize) -> bool {
        self.delaunay_edge[(self.slot(p) + 1) % 3]
    }

    fn delaunay_edge_ccw(&self, p: usize) -> bool {
        self.delaunay_edge[(self.slot(p) + 2) % 3]
    }

    fn set_delaunay_edge_cw(&mut self, p: usize, value: bool) {
        self.delaunay_edge[(self.slot(p) + 1) % 3] = value;
    }

    fn set_delaunay_edge_ccw(&mut self, p: usize, value: bool) {
        self.delaunay_edge[(self.slot(p) + 2) % 3] = value;
    }

    /// `Legalize(opoint, npoint)`: rotated so `npoint` replaces the point
    /// after `opoint`.
    fn legalize(&mut self, opoint: usize, npoint: usize) -> Outcome<()> {
        let [a, b, c] = self.points;
        if opoint == a {
            self.points = [c, a, npoint];
        } else if opoint == b {
            self.points = [npoint, a, b];
        } else if opoint == c {
            self.points = [c, npoint, b];
        } else {
            return not_a_corner();
        }
        Ok(())
    }
}

/// An advancing front node.
#[derive(Clone, Copy)]
struct Node {
    point: usize,
    triangle: Option<usize>,
    next: Option<usize>,
    prev: Option<usize>,
    value: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Orientation {
    Cw,
    Ccw,
    Collinear,
}

/// Which way an edge event fills: towards the next nodes or the previous.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Right,
    Left,
}

impl Side {
    /// Whether `a` is still short of `b` going this way.
    fn short_of(self, a: f64, b: f64) -> bool {
        match self {
            Side::Right => a < b,
            Side::Left => a > b,
        }
    }

    /// The turn that means "below the edge" this way.
    fn turn(self) -> Orientation {
        match self {
            Side::Right => Orientation::Ccw,
            Side::Left => Orientation::Cw,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Basin {
    left_node: Option<usize>,
    bottom_node: Option<usize>,
    right_node: Option<usize>,
    width: f64,
    left_highest: bool,
}

/// `Orient2d`.
fn orient_2d(pa: Point, pb: Point, pc: Point) -> Orientation {
    let detleft = (pa.x - pc.x) * (pb.y - pc.y);
    let detright = (pa.y - pc.y) * (pb.x - pc.x);
    let value = detleft - detright;
    if value == 0.0 {
        Orientation::Collinear
    } else if value > 0.0 {
        Orientation::Ccw
    } else {
        Orientation::Cw
    }
}

/// `InScanArea`.
fn in_scan_area(pa: Point, pb: Point, pc: Point, pd: Point) -> bool {
    let oadb = (pa.x - pb.x) * (pd.y - pb.y) - (pd.x - pb.x) * (pa.y - pb.y);
    if oadb >= -EPSILON {
        return false;
    }
    let oadc = (pa.x - pc.x) * (pd.y - pc.y) - (pd.x - pc.x) * (pa.y - pc.y);
    oadc > EPSILON
}

/// `Incircle`: whether `pd` is inside the circle through the others.
fn incircle(pa: Point, pb: Point, pc: Point, pd: Point) -> bool {
    let adx = pa.x - pd.x;
    let ady = pa.y - pd.y;
    let bdx = pb.x - pd.x;
    let bdy = pb.y - pd.y;
    let oabd = adx * bdy - bdx * ady;
    if oabd <= 0.0 {
        return false;
    }
    let cdx = pc.x - pd.x;
    let cdy = pc.y - pd.y;
    let ocad = cdx * ady - adx * cdy;
    if ocad <= 0.0 {
        return false;
    }
    let bdxcdy = bdx * cdy;
    let cdxbdy = cdx * bdy;
    let alift = adx * adx + ady * ady;
    let blift = bdx * bdx + bdy * bdy;
    let clift = cdx * cdx + cdy * cdy;
    let det = alift * (bdxcdy - cdxbdy) + blift * ocad + clift * oabd;
    det > 0.0
}

/// `Sweep::Angle`: the angle between `pa` and `pb` seen from `origin`.
fn angle(origin: Point, pa: Point, pb: Point) -> f64 {
    let ax = pa.x - origin.x;
    let ay = pa.y - origin.y;
    let bx = pb.x - origin.x;
    let by = pb.y - origin.y;
    let x = ax * by - ay * bx;
    let y = ax * bx + ay * by;
    x.atan2(y)
}

/// The `assert(0)` after a lookup of a point that isn't a corner (or an
/// edge that isn't one).
fn not_a_corner<T>() -> Outcome<T> {
    Err(Failure::Process(crate::scene::assertion_failed("0")))
}

fn missing<T>(value: Option<T>, what: &str) -> Outcome<T> {
    value.ok_or_else(|| Failure::undefined(what))
}

fn runtime_error(message: &str) -> Failure {
    Failure::Std(message.to_string())
}

/// `p2t::CDT` with its `SweepContext` and `Sweep`.
pub(super) struct Cdt {
    vertices: Vec<Vertex>,
    edges: Vec<Edge>,
    triangles: Vec<Triangle>,
    nodes: Vec<Node>,
    /// `points_`: the polygon's and holes' points, sorted when the
    /// triangulation starts.
    points: Vec<usize>,
    /// `triangles_`: the interior triangles `MeshClean` found.
    interior: Vec<usize>,
    /// The head and tail vertices below the points.
    head: usize,
    /// The front's first node.
    front_head: usize,
    search_node: Option<usize>,
    basin: Basin,
    constrained_edge: Option<usize>,
    edge_event_right: bool,
    depth: usize,
}

impl Cdt {
    /// `CDT(polyline)`: the outline's points and edges. Two equal points
    /// in a row throw.
    pub(super) fn new(polyline: Vec<Point>) -> Outcome<Cdt> {
        let mut cdt = Cdt {
            vertices: Vec::new(),
            edges: Vec::new(),
            triangles: Vec::new(),
            nodes: Vec::new(),
            points: Vec::new(),
            interior: Vec::new(),
            head: 0,
            front_head: 0,
            search_node: None,
            basin: Basin::default(),
            constrained_edge: None,
            edge_event_right: false,
            depth: 0,
        };
        let outline = cdt.add_vertices(polyline);
        cdt.points = outline.clone();
        cdt.init_edges(&outline)?;
        Ok(cdt)
    }

    /// `AddHole`: the hole's edges, then its points.
    pub(super) fn add_hole(&mut self, polyline: Vec<Point>) -> Outcome<()> {
        let hole = self.add_vertices(polyline);
        self.init_edges(&hole)?;
        self.points.extend(hole);
        Ok(())
    }

    /// `GetTriangles`: the triangles inside the polygon, three points each.
    pub(super) fn triangles(&self) -> Vec<[Point; 3]> {
        let mut result = Vec::with_capacity(self.interior.len());
        for &triangle in &self.interior {
            let [a, b, c] = self.triangles[triangle].points;
            result.push([self.vertices[a].point, self.vertices[b].point, self.vertices[c].point]);
        }
        result
    }

    fn add_vertices(&mut self, polyline: Vec<Point>) -> Vec<usize> {
        let first = self.vertices.len();
        for point in polyline {
            self.vertices.push(Vertex { point, edge_list: Vec::new() });
        }
        (first..self.vertices.len()).collect()
    }

    /// `InitEdges`: an edge from each point to the next (the last to the
    /// first).
    fn init_edges(&mut self, polyline: &[usize]) -> Outcome<()> {
        for (i, &p1) in polyline.iter().enumerate() {
            let p2 = polyline[(i + 1) % polyline.len()];
            self.new_edge(p1, p2)?;
        }
        Ok(())
    }

    /// `Edge(p1, p2)`, listed at its upper end.
    fn new_edge(&mut self, p1: usize, p2: usize) -> Outcome<()> {
        let a = self.vertices[p1].point;
        let b = self.vertices[p2].point;
        let (p, q) = if a.y > b.y || (a.y == b.y && a.x > b.x) {
            (p2, p1)
        } else if a.y == b.y && a.x == b.x {
            return Err(runtime_error("Edge::Edge: p1 == p2"));
        } else {
            (p1, p2)
        };
        let edge = self.edges.len();
        self.edges.push(Edge { p, q });
        self.vertices[q].edge_list.push(edge);
        Ok(())
    }

    fn point(&self, vertex: usize) -> Point {
        self.vertices[vertex].point
    }

    fn node_point(&self, node: usize) -> Point {
        self.point(self.nodes[node].point)
    }

    fn next(&self, node: usize) -> Outcome<usize> {
        missing(self.nodes[node].next, "poly2tri read past the end of its advancing front")
    }

    fn prev(&self, node: usize) -> Outcome<usize> {
        missing(self.nodes[node].prev, "poly2tri read past the start of its advancing front")
    }

    fn step(&self, node: usize, side: Side) -> Outcome<usize> {
        match side {
            Side::Right => self.next(node),
            Side::Left => self.prev(node),
        }
    }

    fn node_triangle(&self, node: usize) -> Outcome<usize> {
        missing(self.nodes[node].triangle, "poly2tri used a front node without a triangle")
    }

    fn new_triangle(&mut self, a: usize, b: usize, c: usize) -> usize {
        self.triangles.push(Triangle::new(a, b, c));
        self.triangles.len() - 1
    }

    fn new_node(&mut self, point: usize, triangle: Option<usize>) -> usize {
        let value = self.point(point).x;
        let node = Node { point, triangle, next: None, prev: None, value };
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// A recursive call, past `MAX_DEPTH` the native stack's end.
    fn nested<T>(&mut self, call: impl FnOnce(&mut Self) -> Outcome<T>) -> Outcome<T> {
        if self.depth >= MAX_DEPTH {
            return Err(Failure::undefined("poly2tri recursed past the stack"));
        }
        self.depth += 1;
        let result = call(self);
        self.depth -= 1;
        result
    }

    /// `Triangle::MarkNeighbor(t)`: both linked across their shared edge.
    fn mark_neighbor(&mut self, triangle: usize, other: usize) -> Outcome<()> {
        let [a, b, c] = self.triangles[triangle].points;
        let shared = if self.triangles[other].contains_both(b, c) {
            Some((0, b, c))
        } else if self.triangles[other].contains_both(a, c) {
            Some((1, a, c))
        } else if self.triangles[other].contains_both(a, b) {
            Some((2, a, b))
        } else {
            None
        };
        if let Some((slot, p1, p2)) = shared {
            self.triangles[triangle].neighbors[slot] = Some(other);
            self.triangles[other].mark_neighbor_across(p1, p2, triangle)?;
        }
        Ok(())
    }

    /// `OppositePoint`: the point of `triangle` across from `t`'s edge
    /// clockwise of `p`.
    fn opposite_point(&self, triangle: usize, t: usize, p: usize) -> Outcome<usize> {
        let cw = self.triangles[t].point_cw(p)?;
        self.triangles[triangle].point_cw(cw)
    }

    /// `Sweep::Triangulate`.
    pub(super) fn triangulate(&mut self) -> Outcome<()> {
        self.init_triangulation();
        self.create_advancing_front();
        self.sweep_points()?;
        self.finalization_polygon()
    }

    /// `InitTriangulation`: head and tail below the points' bounds, the
    /// points sorted by y then x (MSVC's sort decides between equals).
    fn init_triangulation(&mut self) {
        let first = self.point(self.points[0]);
        let (mut xmax, mut xmin, mut ymax, mut ymin) = (first.x, first.x, first.y, first.y);
        for &vertex in &self.points {
            let point = self.vertices[vertex].point;
            if point.x > xmax {
                xmax = point.x;
            }
            if point.x < xmin {
                xmin = point.x;
            }
            if point.y > ymax {
                ymax = point.y;
            }
            if point.y < ymin {
                ymin = point.y;
            }
        }
        let dx = ALPHA * (xmax - xmin);
        let dy = ALPHA * (ymax - ymin);
        let head = Point::new(xmin - dx, ymin - dy);
        let tail = Point::new(xmax + dx, ymin - dy);
        self.head = self.add_vertices(vec![head, tail])[0];
        let vertices = &self.vertices;
        msvc::sort(&mut self.points, |&a, &b| {
            let a = vertices[a].point;
            let b = vertices[b].point;
            a.y < b.y || (a.y == b.y && a.x < b.x)
        });
    }

    /// `CreateAdvancingFront`: the first point under the head-tail span.
    fn create_advancing_front(&mut self) {
        let head = self.head;
        let tail = self.head + 1;
        let triangle = self.new_triangle(self.points[0], head, tail);
        let [middle_point, head_point, tail_point] = self.triangles[triangle].points;
        let af_head = self.new_node(head_point, Some(triangle));
        let af_middle = self.new_node(middle_point, Some(triangle));
        let af_tail = self.new_node(tail_point, None);
        self.front_head = af_head;
        self.search_node = Some(af_head);
        self.nodes[af_head].next = Some(af_middle);
        self.nodes[af_middle].next = Some(af_tail);
        self.nodes[af_middle].prev = Some(af_head);
        self.nodes[af_tail].prev = Some(af_middle);
    }

    /// `SweepPoints`: each point after the first joins the front, then
    /// its edges are made constrained.
    fn sweep_points(&mut self) -> Outcome<()> {
        for index in 1..self.points.len() {
            let point = self.points[index];
            let node = self.point_event(point)?;
            for edge_index in 0..self.vertices[point].edge_list.len() {
                let edge = self.vertices[point].edge_list[edge_index];
                self.edge_event(edge, node)?;
            }
        }
        Ok(())
    }

    /// `FinalizationPolygon`: from a constrained edge at the front's
    /// first point, the triangles inside.
    fn finalization_polygon(&mut self) -> Outcome<()> {
        let head = self.nodes[self.front_head].next;
        let first = missing(head, "poly2tri's front ended at its head")?;
        let mut triangle = self.nodes[first].triangle;
        let point = self.nodes[first].point;
        let mut steps = 0usize;
        while let Some(t) = triangle
            && !self.triangles[t].constrained_edge_cw(point)
        {
            triangle = self.triangles[t].neighbor_ccw(point);
            steps += 1;
            if steps > self.triangles.len() {
                return Err(Failure::endless("poly2tri circled a point without a constrained edge"));
            }
        }
        if let Some(t) = triangle {
            self.mesh_clean(t);
        }
        Ok(())
    }

    /// `MeshClean`: the triangles reachable without crossing a
    /// constrained edge, depth first.
    fn mesh_clean(&mut self, triangle: usize) {
        let mut stack = vec![Some(triangle)];
        while let Some(entry) = stack.pop() {
            let Some(t) = entry else {
                continue;
            };
            if self.triangles[t].interior {
                continue;
            }
            self.triangles[t].interior = true;
            self.interior.push(t);
            for i in 0..3 {
                if !self.triangles[t].constrained_edge[i] {
                    stack.push(self.triangles[t].neighbors[i]);
                }
            }
        }
    }

    /// `AdvancingFront::LocateNode`: the node at or left of `x`, from the
    /// last search.
    fn locate_node(&mut self, x: f64) -> Outcome<Option<usize>> {
        let mut node = missing(self.search_node, "poly2tri searched its front from nowhere")?;
        if x < self.nodes[node].value {
            while let Some(prev) = self.nodes[node].prev {
                node = prev;
                if x >= self.nodes[node].value {
                    self.search_node = Some(node);
                    return Ok(Some(node));
                }
            }
        } else {
            while let Some(next) = self.nodes[node].next {
                node = next;
                if x < self.nodes[node].value {
                    let prev = self.nodes[node].prev;
                    self.search_node = prev;
                    return Ok(prev);
                }
            }
        }
        Ok(None)
    }

    /// `AdvancingFront::LocatePoint`: the node of a point, from the last
    /// search.
    fn locate_point(&mut self, point: usize) -> Outcome<Option<usize>> {
        let px = self.point(point).x;
        let start = missing(self.search_node, "poly2tri searched its front from nowhere")?;
        let mut node = Some(start);
        let nx = self.node_point(start).x;
        if px == nx {
            if point != self.nodes[start].point {
                let prev = self.prev(start)?;
                if point == self.nodes[prev].point {
                    node = Some(prev);
                } else {
                    let next = self.next(start)?;
                    if point != self.nodes[next].point {
                        return not_a_corner();
                    }
                    node = Some(next);
                }
            }
        } else if px < nx {
            node = self.nodes[start].prev;
            while let Some(current) = node
                && point != self.nodes[current].point
            {
                node = self.nodes[current].prev;
            }
        } else {
            node = self.nodes[start].next;
            while let Some(current) = node
                && point != self.nodes[current].point
            {
                node = self.nodes[current].next;
            }
        }
        if node.is_some() {
            self.search_node = node;
        }
        Ok(node)
    }

    /// `MapTriangleToNodes`: the triangle becomes the front's triangle at
    /// each of its open edges.
    fn map_triangle_to_nodes(&mut self, triangle: usize) -> Outcome<()> {
        for i in 0..3 {
            if self.triangles[triangle].neighbors[i].is_some() {
                continue;
            }
            let corner = self.triangles[triangle].points[i];
            let cw = self.triangles[triangle].point_cw(corner)?;
            if let Some(node) = self.locate_point(cw)? {
                self.nodes[node].triangle = Some(triangle);
            }
        }
        Ok(())
    }

    /// `PointEvent`: a triangle from the point to the front below it, the
    /// front filled around it.
    fn point_event(&mut self, point: usize) -> Outcome<usize> {
        let x = self.point(point).x;
        let Some(node) = self.locate_node(x)? else {
            return Err(runtime_error("PointEvent - null node"));
        };
        if self.nodes[node].next.is_none() {
            return Err(runtime_error("PointEvent - null node"));
        }
        let new_node = self.new_front_triangle(point, node)?;
        if x <= self.node_point(node).x + EPSILON {
            self.fill(node)?;
        }
        self.fill_advancing_front(new_node)?;
        Ok(new_node)
    }

    /// `NewFrontTriangle`.
    fn new_front_triangle(&mut self, point: usize, node: usize) -> Outcome<usize> {
        let next = self.next(node)?;
        let triangle = self.new_triangle(point, self.nodes[node].point, self.nodes[next].point);
        let below = self.node_triangle(node)?;
        self.mark_neighbor(triangle, below)?;
        let new_node = self.new_node(point, None);
        self.nodes[new_node].next = Some(next);
        self.nodes[new_node].prev = Some(node);
        self.nodes[next].prev = Some(new_node);
        self.nodes[node].next = Some(new_node);
        if !self.legalize(triangle)? {
            self.map_triangle_to_nodes(triangle)?;
        }
        Ok(new_node)
    }

    /// `Fill`: a triangle over the node, which leaves the front.
    fn fill(&mut self, node: usize) -> Outcome<()> {
        let prev = self.prev(node)?;
        let next = self.next(node)?;
        let triangle = self.new_triangle(self.nodes[prev].point, self.nodes[node].point, self.nodes[next].point);
        let prev_triangle = self.node_triangle(prev)?;
        self.mark_neighbor(triangle, prev_triangle)?;
        let node_triangle = self.node_triangle(node)?;
        self.mark_neighbor(triangle, node_triangle)?;
        self.nodes[prev].next = Some(next);
        self.nodes[next].prev = Some(prev);
        if !self.legalize(triangle)? {
            self.map_triangle_to_nodes(triangle)?;
        }
        Ok(())
    }

    /// `FillAdvancingFront`: holes either side of the new node filled
    /// (large ones left), then a basin to its right.
    fn fill_advancing_front(&mut self, n: usize) -> Outcome<()> {
        let mut node = self.nodes[n].next;
        while let Some(current) = node
            && self.nodes[current].next.is_some()
        {
            if self.large_hole_dont_fill(current)? {
                break;
            }
            self.fill(current)?;
            node = self.nodes[current].next;
        }

        node = self.nodes[n].prev;
        while let Some(current) = node
            && self.nodes[current].prev.is_some()
        {
            if self.large_hole_dont_fill(current)? {
                break;
            }
            self.fill(current)?;
            node = self.nodes[current].prev;
        }

        if let Some(next) = self.nodes[n].next
            && let Some(next_next) = self.nodes[next].next
        {
            let here = self.node_point(n);
            let there = self.node_point(next_next);
            let basin_angle = (here.y - there.y).atan2(here.x - there.x);
            if basin_angle < PI_3DIV4 {
                self.fill_basin(n)?;
            }
        }
        Ok(())
    }

    /// `LargeHole_DontFill`.
    fn large_hole_dont_fill(&self, node: usize) -> Outcome<bool> {
        let next_node = self.next(node)?;
        let prev_node = self.prev(node)?;
        let origin = self.node_point(node);
        let next = self.node_point(next_node);
        let prev = self.node_point(prev_node);
        let first = angle(origin, next, prev);
        if !(first > PI_DIV2 || first < -PI_DIV2) {
            return Ok(false);
        }
        if first < 0.0 {
            return Ok(true);
        }
        let exceeds_or_negative = |pa: Point, pb: Point| {
            let value = angle(origin, pa, pb);
            value > PI_DIV2 || value < 0.0
        };
        if let Some(next_2) = self.nodes[next_node].next
            && !exceeds_or_negative(self.node_point(next_2), prev)
        {
            return Ok(false);
        }
        if let Some(prev_2) = self.nodes[prev_node].prev
            && !exceeds_or_negative(next, self.node_point(prev_2))
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// `FillBasin`: a dip in the front right of the node filled.
    fn fill_basin(&mut self, node: usize) -> Outcome<()> {
        let next = self.next(node)?;
        let next_next = self.next(next)?;
        let turn = orient_2d(self.node_point(node), self.node_point(next), self.node_point(next_next));
        let left = if turn == Orientation::Ccw { next_next } else { next };
        self.basin.left_node = Some(left);

        let mut bottom = left;
        while let Some(after) = self.nodes[bottom].next
            && self.node_point(bottom).y >= self.node_point(after).y
        {
            bottom = after;
        }
        self.basin.bottom_node = Some(bottom);
        if bottom == left {
            return Ok(());
        }

        let mut right = bottom;
        while let Some(after) = self.nodes[right].next
            && self.node_point(right).y < self.node_point(after).y
        {
            right = after;
        }
        self.basin.right_node = Some(right);
        if right == bottom {
            return Ok(());
        }

        self.basin.width = self.node_point(right).x - self.node_point(left).x;
        self.basin.left_highest = self.node_point(left).y > self.node_point(right).y;
        self.fill_basin_req(bottom)
    }

    /// `FillBasinReq`: filled from the bottom up, towards the lower side.
    fn fill_basin_req(&mut self, node: usize) -> Outcome<()> {
        if self.is_shallow(node)? {
            return Ok(());
        }
        self.fill(node)?;
        let prev = self.prev(node)?;
        let next = self.next(node)?;
        let next_node = if Some(prev) == self.basin.left_node && Some(next) == self.basin.right_node {
            return Ok(());
        } else if Some(prev) == self.basin.left_node {
            let after = self.next(next)?;
            let turn = orient_2d(self.node_point(node), self.node_point(next), self.node_point(after));
            if turn == Orientation::Cw {
                return Ok(());
            }
            next
        } else if Some(next) == self.basin.right_node {
            let before = self.prev(prev)?;
            let turn = orient_2d(self.node_point(node), self.node_point(prev), self.node_point(before));
            if turn == Orientation::Ccw {
                return Ok(());
            }
            prev
        } else if self.node_point(prev).y < self.node_point(next).y {
            prev
        } else {
            next
        };
        self.nested(|cdt| cdt.fill_basin_req(next_node))
    }

    /// `IsShallow`.
    fn is_shallow(&self, node: usize) -> Outcome<bool> {
        let side = if self.basin.left_highest { self.basin.left_node } else { self.basin.right_node };
        let side = missing(side, "poly2tri measured a basin without sides")?;
        let height = self.node_point(side).y - self.node_point(node).y;
        Ok(self.basin.width > height)
    }

    /// `EdgeEvent(edge, node)`: the edge made constrained, the front
    /// filled above it first.
    fn edge_event(&mut self, edge: usize, node: usize) -> Outcome<()> {
        self.constrained_edge = Some(edge);
        let Edge { p, q } = self.edges[edge];
        self.edge_event_right = self.point(p).x > self.point(q).x;
        let triangle = self.node_triangle(node)?;
        if self.is_edge_side_of_triangle(triangle, p, q) {
            return Ok(());
        }
        let side = if self.edge_event_right { Side::Right } else { Side::Left };
        self.fill_above_edge_event(edge, node, side)?;
        let Edge { p, q } = self.edges[edge];
        let triangle = self.nodes[node].triangle;
        self.edge_event_points(p, q, triangle, q)
    }

    /// `IsEdgeSideOfTriangle`: when ep-eq is an edge of the triangle, it
    /// is marked constrained on both sides.
    fn is_edge_side_of_triangle(&mut self, triangle: usize, ep: usize, eq: usize) -> bool {
        let Some(index) = self.triangles[triangle].edge_index(ep, eq) else {
            return false;
        };
        self.triangles[triangle].constrained_edge[index] = true;
        if let Some(neighbor) = self.triangles[triangle].neighbors[index] {
            self.triangles[neighbor].mark_constrained_edge(ep, eq);
        }
        true
    }

    /// `EdgeEvent(ep, eq, triangle, point)`: from the triangle at `point`
    /// towards `ep`, flipping what the edge crosses.
    fn edge_event_points(&mut self, ep: usize, eq: usize, triangle: Option<usize>, point: usize) -> Outcome<()> {
        let Some(triangle) = triangle else {
            return Err(runtime_error("EdgeEvent - null triangle"));
        };
        if self.is_edge_side_of_triangle(triangle, ep, eq) {
            return Ok(());
        }

        let p1 = self.triangles[triangle].point_ccw(point)?;
        let o1 = orient_2d(self.point(eq), self.point(p1), self.point(ep));
        if o1 == Orientation::Collinear {
            return self.collinear_edge_event(ep, eq, triangle, point, p1);
        }
        let p2 = self.triangles[triangle].point_cw(point)?;
        let o2 = orient_2d(self.point(eq), self.point(p2), self.point(ep));
        if o2 == Orientation::Collinear {
            return self.collinear_edge_event(ep, eq, triangle, point, p2);
        }

        if o1 == o2 {
            let next = if o1 == Orientation::Cw {
                self.triangles[triangle].neighbor_ccw(point)
            } else {
                self.triangles[triangle].neighbor_cw(point)
            };
            return self.nested(|cdt| cdt.edge_event_points(ep, eq, next, point));
        }
        self.nested(|cdt| cdt.flip_edge_event(ep, eq, triangle, point))
    }

    /// The edge runs through a triangle's corner: constrained up to it,
    /// continued from there.
    fn collinear_edge_event(&mut self, ep: usize, eq: usize, triangle: usize, point: usize, corner: usize) -> Outcome<()> {
        if !self.triangles[triangle].contains_both(eq, corner) {
            return Err(runtime_error("EdgeEvent - collinear points not supported"));
        }
        self.triangles[triangle].mark_constrained_edge(eq, corner);
        let edge = missing(self.constrained_edge, "poly2tri lost its constrained edge")?;
        self.edges[edge].q = corner;
        let next = self.triangles[triangle].neighbor_across(point);
        self.nested(|cdt| cdt.edge_event_points(ep, corner, next, corner))
    }

    /// `FillRightAboveEdgeEvent` / `FillLeftAboveEdgeEvent`.
    fn fill_above_edge_event(&mut self, edge: usize, mut node: usize, side: Side) -> Outcome<()> {
        loop {
            let ahead = self.step(node, side)?;
            let Edge { p, q } = self.edges[edge];
            if !side.short_of(self.node_point(ahead).x, self.point(p).x) {
                return Ok(());
            }
            if orient_2d(self.point(q), self.node_point(ahead), self.point(p)) != side.turn() {
                node = ahead;
                continue;
            }
            let before = self.triangles.len();
            self.fill_below_edge_event(edge, node, side)?;
            if self.triangles.len() == before {
                return Err(Failure::endless("poly2tri filled nothing below a constrained edge"));
            }
        }
    }

    /// `FillRightBelowEdgeEvent` / `FillLeftBelowEdgeEvent`.
    fn fill_below_edge_event(&mut self, edge: usize, node: usize, side: Side) -> Outcome<()> {
        let p = self.edges[edge].p;
        if !side.short_of(self.node_point(node).x, self.point(p).x) {
            return Ok(());
        }
        let ahead = self.step(node, side)?;
        let beyond = self.step(ahead, side)?;
        let turn = orient_2d(self.node_point(node), self.node_point(ahead), self.node_point(beyond));
        if turn == side.turn() {
            return self.nested(|cdt| cdt.fill_concave_edge_event(edge, node, side));
        }
        let before = self.triangles.len();
        self.nested(|cdt| cdt.fill_convex_edge_event(edge, node, side))?;
        if self.triangles.len() == before {
            // The same call again, for ever.
            return Err(Failure::undefined("poly2tri recursed past the stack"));
        }
        self.nested(|cdt| cdt.fill_below_edge_event(edge, node, side))
    }

    /// `FillRightConcaveEdgeEvent` / `FillLeftConcaveEdgeEvent`.
    fn fill_concave_edge_event(&mut self, edge: usize, node: usize, side: Side) -> Outcome<()> {
        let ahead = self.step(node, side)?;
        self.fill(ahead)?;
        let ahead = self.step(node, side)?;
        let Edge { p, q } = self.edges[edge];
        if self.nodes[ahead].point == p {
            return Ok(());
        }
        if orient_2d(self.point(q), self.node_point(ahead), self.point(p)) != side.turn() {
            return Ok(());
        }
        let beyond = self.step(ahead, side)?;
        let turn = orient_2d(self.node_point(node), self.node_point(ahead), self.node_point(beyond));
        if turn == side.turn() {
            return self.nested(|cdt| cdt.fill_concave_edge_event(edge, node, side));
        }
        Ok(())
    }

    /// `FillRightConvexEdgeEvent` / `FillLeftConvexEdgeEvent`.
    fn fill_convex_edge_event(&mut self, edge: usize, node: usize, side: Side) -> Outcome<()> {
        let ahead = self.step(node, side)?;
        let beyond = self.step(ahead, side)?;
        let further = self.step(beyond, side)?;
        let turn = orient_2d(self.node_point(ahead), self.node_point(beyond), self.node_point(further));
        if turn == side.turn() {
            return self.nested(|cdt| cdt.fill_concave_edge_event(edge, ahead, side));
        }
        let Edge { p, q } = self.edges[edge];
        if orient_2d(self.point(q), self.node_point(beyond), self.point(p)) == side.turn() {
            return self.nested(|cdt| cdt.fill_convex_edge_event(edge, ahead, side));
        }
        Ok(())
    }

    /// `FlipEdgeEvent`: the triangle pair across `p` rotated while the
    /// edge crosses them.
    fn flip_edge_event(&mut self, ep: usize, eq: usize, t: usize, p: usize) -> Outcome<()> {
        let Some(ot) = self.triangles[t].neighbor_across(p) else {
            return Err(runtime_error("FlipEdgeEvent - null neighbor across"));
        };
        let op = self.opposite_point(ot, t, p)?;
        let ccw = self.triangles[t].point_ccw(p)?;
        let cw = self.triangles[t].point_cw(p)?;

        if !in_scan_area(self.point(p), self.point(ccw), self.point(cw), self.point(op)) {
            let new_p = self.next_flip_point(ep, eq, ot, op)?;
            self.nested(|cdt| cdt.flip_scan_edge_event(ep, eq, t, ot, new_p))?;
            return self.nested(|cdt| cdt.edge_event_points(ep, eq, Some(t), p));
        }

        self.rotate_triangle_pair(t, p, ot, op)?;
        self.map_triangle_to_nodes(t)?;
        self.map_triangle_to_nodes(ot)?;
        // Points compare by value here.
        if self.point(p) == self.point(eq) && self.point(op) == self.point(ep) {
            let edge = missing(self.constrained_edge, "poly2tri lost its constrained edge")?;
            let Edge { p: edge_p, q: edge_q } = self.edges[edge];
            if self.point(eq) == self.point(edge_q) && self.point(ep) == self.point(edge_p) {
                self.triangles[t].mark_constrained_edge(ep, eq);
                self.triangles[ot].mark_constrained_edge(ep, eq);
                self.legalize(t)?;
                self.legalize(ot)?;
            }
            return Ok(());
        }
        let turn = orient_2d(self.point(eq), self.point(op), self.point(ep));
        let next = self.next_flip_triangle(turn, t, ot, p, op)?;
        self.nested(|cdt| cdt.flip_edge_event(ep, eq, next, p))
    }

    /// `NextFlipTriangle`: the pair's other triangle legalized; the one
    /// to keep flipping.
    fn next_flip_triangle(&mut self, turn: Orientation, t: usize, ot: usize, p: usize, op: usize) -> Outcome<usize> {
        let (legalized, kept) = if turn == Orientation::Ccw { (ot, t) } else { (t, ot) };
        let edge_index = self.triangles[legalized].edge_index(p, op);
        self.triangles[legalized].set_delaunay_edge_at(edge_index, true);
        self.legalize(legalized)?;
        self.triangles[legalized].delaunay_edge = [false; 3];
        Ok(kept)
    }

    /// `NextFlipPoint`.
    fn next_flip_point(&self, ep: usize, eq: usize, ot: usize, op: usize) -> Outcome<usize> {
        match orient_2d(self.point(eq), self.point(op), self.point(ep)) {
            Orientation::Cw => self.triangles[ot].point_ccw(op),
            Orientation::Ccw => self.triangles[ot].point_cw(op),
            Orientation::Collinear => Err(runtime_error("[Unsupported] Opposing point on constrained edge")),
        }
    }

    /// `FlipScanEdgeEvent`: across the triangles the edge passes until
    /// one can be flipped.
    fn flip_scan_edge_event(&mut self, ep: usize, eq: usize, flip_triangle: usize, t: usize, p: usize) -> Outcome<()> {
        let Some(ot) = self.triangles[t].neighbor_across(p) else {
            return Err(runtime_error("FlipScanEdgeEvent - null neighbor across"));
        };
        // The lookups assert before the null checks after them could throw.
        let op = self.opposite_point(ot, t, p)?;
        let p1 = self.triangles[flip_triangle].point_ccw(eq)?;
        let p2 = self.triangles[flip_triangle].point_cw(eq)?;
        if in_scan_area(self.point(eq), self.point(p1), self.point(p2), self.point(op)) {
            return self.nested(|cdt| cdt.flip_edge_event(eq, op, ot, op));
        }
        let new_p = self.next_flip_point(ep, eq, ot, op)?;
        self.nested(|cdt| cdt.flip_scan_edge_event(ep, eq, flip_triangle, ot, new_p))
    }

    /// `Legalize`: edges whose opposite point is inside the circumcircle
    /// flipped, recursively. Whether anything was.
    fn legalize(&mut self, t: usize) -> Outcome<bool> {
        for i in 0..3 {
            if self.triangles[t].delaunay_edge[i] {
                continue;
            }
            let Some(ot) = self.triangles[t].neighbors[i] else {
                continue;
            };
            let p = self.triangles[t].points[i];
            let op = self.opposite_point(ot, t, p)?;
            let oi = self.triangles[ot].index(op)?;
            let other = self.triangles[ot];
            if other.constrained_edge[oi] || other.delaunay_edge[oi] {
                self.triangles[t].constrained_edge[i] = other.constrained_edge[oi];
                continue;
            }
            let ccw = self.triangles[t].point_ccw(p)?;
            let cw = self.triangles[t].point_cw(p)?;
            if !incircle(self.point(p), self.point(ccw), self.point(cw), self.point(op)) {
                continue;
            }
            self.triangles[t].delaunay_edge[i] = true;
            self.triangles[ot].delaunay_edge[oi] = true;
            self.rotate_triangle_pair(t, p, ot, op)?;
            if !self.nested(|cdt| cdt.legalize(t))? {
                self.map_triangle_to_nodes(t)?;
            }
            if !self.nested(|cdt| cdt.legalize(ot))? {
                self.map_triangle_to_nodes(ot)?;
            }
            self.triangles[t].delaunay_edge[i] = false;
            self.triangles[ot].delaunay_edge[oi] = false;
            return Ok(true);
        }
        Ok(false)
    }

    /// `RotateTrianglePair`: the shared edge flipped, flags and neighbours
    /// carried over.
    fn rotate_triangle_pair(&mut self, t: usize, p: usize, ot: usize, op: usize) -> Outcome<()> {
        let first = self.triangles[t];
        let second = self.triangles[ot];
        let n1 = first.neighbor_ccw(p);
        let n2 = first.neighbor_cw(p);
        let n3 = second.neighbor_ccw(op);
        let n4 = second.neighbor_cw(op);
        let ce1 = first.constrained_edge_ccw(p);
        let ce2 = first.constrained_edge_cw(p);
        let ce3 = second.constrained_edge_ccw(op);
        let ce4 = second.constrained_edge_cw(op);
        let de1 = first.delaunay_edge_ccw(p);
        let de2 = first.delaunay_edge_cw(p);
        let de3 = second.delaunay_edge_ccw(op);
        let de4 = second.delaunay_edge_cw(op);

        self.triangles[t].legalize(p, op)?;
        self.triangles[ot].legalize(op, p)?;

        self.triangles[ot].set_delaunay_edge_ccw(p, de1);
        self.triangles[t].set_delaunay_edge_cw(p, de2);
        self.triangles[t].set_delaunay_edge_ccw(op, de3);
        self.triangles[ot].set_delaunay_edge_cw(op, de4);

        self.triangles[ot].set_constrained_edge_ccw(p, ce1);
        self.triangles[t].set_constrained_edge_cw(p, ce2);
        self.triangles[t].set_constrained_edge_ccw(op, ce3);
        self.triangles[ot].set_constrained_edge_cw(op, ce4);

        self.triangles[t].neighbors = [None; 3];
        self.triangles[ot].neighbors = [None; 3];
        if let Some(n1) = n1 {
            self.mark_neighbor(ot, n1)?;
        }
        if let Some(n2) = n2 {
            self.mark_neighbor(t, n2)?;
        }
        if let Some(n3) = n3 {
            self.mark_neighbor(t, n3)?;
        }
        if let Some(n4) = n4 {
            self.mark_neighbor(ot, n4)?;
        }
        self.mark_neighbor(t, ot)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, size: f64) -> Vec<Point> {
        vec![Point::new(x, y), Point::new(x + size, y), Point::new(x + size, y + size), Point::new(x, y + size)]
    }

    fn area(triangle: &[Point; 3]) -> f64 {
        let [a, b, c] = triangle;
        ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() * 0.5
    }

    #[test]
    fn a_square_is_two_triangles() {
        let mut cdt = Cdt::new(square(0.0, 0.0, 1.0)).unwrap();
        cdt.triangulate().unwrap();
        let triangles = cdt.triangles();
        assert_eq!(triangles.len(), 2);
        let total: f64 = triangles.iter().map(area).sum();
        assert_eq!(total, 1.0);
    }

    #[test]
    fn a_hole_is_left_out() {
        let mut cdt = Cdt::new(square(0.0, 0.0, 1.0)).unwrap();
        cdt.add_hole(square(0.25, 0.25, 0.5)).unwrap();
        cdt.triangulate().unwrap();
        let triangles = cdt.triangles();
        assert_eq!(triangles.len(), 8);
        let total: f64 = triangles.iter().map(area).sum();
        assert!((total - 0.75).abs() < 1e-12, "{total}");
    }

    #[test]
    fn repeated_points_throw() {
        let outline = vec![Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(1.0, 1.0)];
        assert!(matches!(Cdt::new(outline), Err(Failure::Std(_))));
    }
}
