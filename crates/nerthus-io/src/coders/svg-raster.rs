//! Cairo's 24.8 paths and 256×15 coverage grid, implemented in Rust.
//!
//! The scan conversion follows `cairo-tor-scan-converter.c` (Glitter).
//! Copyright (c) 2008 M Joonas Pihlaja; (c) 2007 David Turner.
//! Permission is hereby granted, free of charge, to any person obtaining a
//! copy of this software and associated documentation files (the "Software"),
//! to deal in the Software without restriction, including without limitation
//! the rights to use, copy, modify, merge, publish, distribute, sublicense,
//! and/or sell copies of the Software, and to permit persons to whom the
//! Software is furnished to do so, subject to the following conditions:
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
//! THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
//! FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
//! IN THE SOFTWARE.

#![allow(non_snake_case)]

use std::collections::BTreeMap;

use resvg::tiny_skia::{Mask, Path, PathSegment, Point, Transform};

const GRID_X: i64 = 256;
const GRID_Y: i64 = 15;

#[derive(Clone, Copy, Default, PartialEq)]
struct FixedPoint {
    x: i64,
    y: i64,
}

impl FixedPoint {
    fn fromPoint(point: Point) -> Self {
        Self {
            x: (point.x as f64 * 256.0).round_ties_even() as i64,
            y: (point.y as f64 * 256.0).round_ties_even() as i64,
        }
    }

    fn midpoint(self, other: Self) -> Self {
        Self { x: self.x + ((other.x - self.x) >> 1), y: self.y + ((other.y - self.y) >> 1) }
    }
}

#[derive(Clone)]
struct Edge {
    top: i64,
    bottom: i64,
    direction: i32,
    x: i64,
    remainder: i64,
    denominator: i64,
    step: i64,
    step_remainder: i64,
    full_step: i64,
    full_remainder: i64,
}

impl Edge {
    fn new(mut start: FixedPoint, mut end: FixedPoint, height: u32) -> Option<Self> {
        let direction = if start.y < end.y { 1 } else { -1 };
        if start.y > end.y {
            std::mem::swap(&mut start, &mut end);
        }
        let top = ((start.y * GRID_Y + 128) >> 8).max(0);
        let bottom = ((end.y * GRID_Y + 128) >> 8).min(height as i64 * GRID_Y);
        if bottom <= top {
            return None;
        }
        let mut edge = Self {
            top, bottom, direction, x: start.x, remainder: 0,
            denominator: 0, step: 0, step_remainder: 0, full_step: 0, full_remainder: 0,
        };
        if start.x == end.x {
            return Some(edge);
        }

        let horizontal = (end.x - start.x) * GRID_X;
        let denominator = (end.y - start.y) * GRID_Y * 512;
        let initial_y = (2 * top + 1) * 256 - start.y * GRID_Y * 2;
        let initial = initial_y as i128 * horizontal as i128;
        edge.denominator = denominator;
        edge.x += initial.div_euclid(denominator as i128) as i64;
        edge.remainder = initial.rem_euclid(denominator as i128) as i64;
        edge.step = horizontal * 512 / denominator;
        edge.step_remainder = horizontal * 512 % denominator;
        edge.full_step = horizontal * 512 * GRID_Y / denominator;
        edge.full_remainder = horizontal * 512 * GRID_Y % denominator;
        Some(edge)
    }

    fn cell(&self) -> i64 {
        self.x + i64::from(self.denominator != 0 && self.remainder >= self.denominator / 2)
    }

    fn advance(&mut self, full: bool) {
        if self.denominator == 0 {
            return;
        }
        let (step, remainder) = if full {
            (self.full_step, self.full_remainder)
        } else {
            (self.step, self.step_remainder)
        };
        self.x += step;
        self.remainder += remainder;
        self.normalize();
    }

    fn normalize(&mut self) {
        if self.remainder < 0 {
            self.x -= 1;
            self.remainder += self.denominator;
        } else if self.remainder >= self.denominator && self.denominator != 0 {
            self.x += 1;
            self.remainder -= self.denominator;
        }
    }

    fn origin(&self) -> (i64, i64) {
        let mut origin = self.clone();
        if self.denominator != 0 {
            origin.x -= self.step / 2;
            origin.remainder -= self.step_remainder / 2;
            origin.normalize();
        }
        (origin.x, origin.remainder)
    }
}

#[derive(Default)]
struct Cell {
    height: i64,
    area: i64,
}

fn addSubspan(cells: &mut BTreeMap<i64, Cell>, start: i64, end: i64) {
    if start == end {
        return;
    }
    let start_cell = cells.entry(start >> 8).or_default();
    start_cell.height += 1;
    start_cell.area += 2 * (start & 255);
    let end_cell = cells.entry(end >> 8).or_default();
    end_cell.height -= 1;
    end_cell.area -= 2 * (end & 255);
}

fn addFullEdge(cells: &mut BTreeMap<i64, Cell>, edge: &mut Edge, sign: i64, width: u32) {
    let mut start = edge.origin();
    edge.advance(true);
    let mut end = edge.origin();
    if end.0 < start.0 {
        std::mem::swap(&mut start, &mut end);
    }
    let first = start.0 >> 8;
    let last = end.0 >> 8;
    if last < 0 {
        cells.entry(-1).or_default().height += sign * GRID_Y;
        return;
    }
    if first >= width as i64 {
        return;
    }
    if first == last {
        let cell = cells.entry(first).or_default();
        cell.height += sign * GRID_Y;
        cell.area += sign * ((start.0 & 255) + (end.0 & 255)) * GRID_Y;
        return;
    }
    let denominator = edge.denominator as i128;
    let distance = (end.0 - start.0) as i128 * denominator + (end.1 - start.1) as i128;
    let start_numerator = start.0 as i128 * denominator + start.1 as i128;
    // Only the total height of cells left of the viewport affects it.
    let clipped_first = first.max(-1);
    let clipped_last = last.min(width as i64);
    let mut previous_y = if first < clipped_first {
        let crossing = clipped_first as i128 * 256 * denominator - start_numerator;
        let height = (crossing * GRID_Y as i128 / distance) as i64;
        cells.entry(clipped_first - 1).or_default().height += sign * height;
        height
    } else {
        0
    };
    for column in clipped_first..=clipped_last {
        let y = if column == last {
            GRID_Y
        } else {
            let crossing = (column + 1) as i128 * 256 * denominator - start_numerator;
            (crossing * GRID_Y as i128 / distance) as i64
        };
        let height = y - previous_y;
        let area = if column == first {
            height * (256 + (start.0 & 255))
        } else if column == last {
            height * (end.0 & 255)
        } else {
            height * 256
        };
        let cell = cells.entry(column).or_default();
        cell.height += sign * height;
        cell.area += sign * area;
        previous_y = y;
    }
}

fn curveFlatEnough(points: &[FixedPoint; 4]) -> bool {
    let delta_x = (points[3].x - points[0].x) as f64;
    let delta_y = (points[3].y - points[0].y) as f64;
    let length_squared = delta_x * delta_x + delta_y * delta_y;
    for point in &points[1..3] {
        let mut offset_x = (point.x - points[0].x) as f64;
        let mut offset_y = (point.y - points[0].y) as f64;
        if length_squared != 0.0 {
            let projection = (offset_x * delta_x + offset_y * delta_y) / length_squared;
            let projection = projection.clamp(0.0, 1.0);
            offset_x -= projection * delta_x;
            offset_y -= projection * delta_y;
        }
        // Cairo's default tolerance is one tenth of a device pixel.
        if offset_x * offset_x + offset_y * offset_y >= 0.01 * 256.0 * 256.0 {
            return false;
        }
    }
    true
}

fn flattenCurve(points: [FixedPoint; 4], output: &mut Vec<FixedPoint>, depth: u32) {
    if depth == 24 || curveFlatEnough(&points) {
        output.push(points[3]);
        return;
    }
    let ab = points[0].midpoint(points[1]);
    let bc = points[1].midpoint(points[2]);
    let cd = points[2].midpoint(points[3]);
    let abc = ab.midpoint(bc);
    let bcd = bc.midpoint(cd);
    let middle = abc.midpoint(bcd);
    let left = [points[0], ab, abc, middle];
    let right = [middle, bcd, cd, points[3]];
    flattenCurve(left, output, depth + 1);
    flattenCurve(right, output, depth + 1);
}

fn flattenPath(path: &Path) -> Vec<Vec<FixedPoint>> {
    let mut contours = Vec::new();
    let mut contour = Vec::new();
    let mut current = FixedPoint::default();
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(point) => {
                if !contour.is_empty() {
                    contours.push(std::mem::take(&mut contour));
                }
                current = FixedPoint::fromPoint(point);
                contour.push(current);
            }
            PathSegment::LineTo(point) => {
                current = FixedPoint::fromPoint(point);
                contour.push(current);
            }
            PathSegment::QuadTo(control, end) => {
                let control = FixedPoint::fromPoint(control);
                let end = FixedPoint::fromPoint(end);
                let first = FixedPoint { x: current.x + (control.x - current.x) * 2 / 3, y: current.y + (control.y - current.y) * 2 / 3 };
                let second = FixedPoint { x: end.x + (control.x - end.x) * 2 / 3, y: end.y + (control.y - end.y) * 2 / 3 };
                let points = [current, first, second, end];
                flattenCurve(points, &mut contour, 0);
                current = end;
            }
            PathSegment::CubicTo(first, second, end) => {
                let end = FixedPoint::fromPoint(end);
                let points = [current, FixedPoint::fromPoint(first), FixedPoint::fromPoint(second), end];
                flattenCurve(points, &mut contour, 0);
                current = end;
            }
            PathSegment::Close => {
                if let Some(&first) = contour.first() {
                    current = first;
                }
            }
        }
    }
    if !contour.is_empty() {
        contours.push(contour);
    }
    contours
}

fn rectangleMask(contours: &[Vec<FixedPoint>], width: u32, height: u32) -> Option<Mask> {
    if contours.len() != 1 {
        return None;
    }
    let contour = &contours[0];
    let count = contour.len() - usize::from(contour.first() == contour.last());
    if count != 4 {
        return None;
    }
    for index in 0..4 {
        let start = contour[index];
        let end = contour[(index + 1) % 4];
        if start.x != end.x && start.y != end.y {
            return None;
        }
    }
    let left = contour.iter().map(|point| point.x).min()?;
    let right = contour.iter().map(|point| point.x).max()?;
    let top = contour.iter().map(|point| point.y).min()?;
    let bottom = contour.iter().map(|point| point.y).max()?;
    let mut mask = Mask::new(width, height)?;
    for row in (top >> 8).max(0)..((bottom + 255) >> 8).min(height as i64) {
        let covered_y = (bottom.min((row + 1) * 256) - top.max(row * 256)).max(0);
        for column in (left >> 8).max(0)..((right + 255) >> 8).min(width as i64) {
            let covered_x = (right.min((column + 1) * 256) - left.max(column * 256)).max(0);
            let area = covered_x * covered_y;
            let alpha = (area >> 8).min(255) as u8;
            mask.data_mut()[row as usize * width as usize + column as usize] = alpha;
        }
    }
    Some(mask)
}

pub(super) fn coverage(path: &Path, transform: Transform, width: u32, height: u32, even_odd: bool) -> Option<Mask> {
    let path = path.clone().transform(transform)?;
    // Keep intermediate arithmetic bounded; the caller can use resvg for
    // exceptional coordinates, without changing the rest of the document.
    if path.points().iter().any(|point| point.x.abs() > 1_000_000.0 || point.y.abs() > 1_000_000.0) {
        return None;
    }
    let contours = flattenPath(&path);
    if let Some(mask) = rectangleMask(&contours, width, height) {
        return Some(mask);
    }
    let mut pending = Vec::new();
    for contour in &contours {
        for index in 0..contour.len() {
            let start = contour[index];
            let end = contour[(index + 1) % contour.len()];
            if let Some(edge) = Edge::new(start, end, height) {
                pending.push(edge);
            }
        }
    }
    pending.sort_by_key(|edge| edge.top);
    let mut pending = pending.into_iter().peekable();
    let mut active: Vec<Edge> = Vec::new();
    let mut mask = Mask::new(width, height)?;
    for row in 0..height {
        let row_top = row as i64 * GRID_Y;
        while pending.peek().is_some_and(|edge| edge.top == row_top) {
            active.push(pending.next().unwrap());
        }
        active.sort_by_key(Edge::cell);
        let starts_inside = pending.peek().is_some_and(|edge| edge.top < row_top + GRID_Y);
        let ends_inside = active.iter().any(|edge| edge.bottom < row_top + GRID_Y);
        let mut previous_cell = i64::MIN;
        let mut intersects = false;
        for edge in &active {
            let mut next = edge.clone();
            next.advance(true);
            let cell = next.cell();
            intersects |= cell < previous_cell;
            previous_cell = cell;
        }
        let mut cells = BTreeMap::new();
        if !starts_inside && !ends_inside && !intersects {
            let mut winding = 0i32;
            let mut left = None;
            for index in 0..active.len() {
                let cell = active[index].cell();
                winding += active[index].direction;
                let inside = if even_odd { winding & 1 != 0 } else { winding != 0 };
                if left.is_none() && inside {
                    left = Some(index);
                }
                let same_next = active.get(index + 1).is_some_and(|edge| edge.cell() == cell);
                if !inside && !same_next && let Some(left_index) = left.take() {
                    addFullEdge(&mut cells, &mut active[left_index], 1, width);
                    addFullEdge(&mut cells, &mut active[index], -1, width);
                    for edge in &mut active[left_index + 1..index] {
                        edge.advance(true);
                    }
                }
            }
        } else {
            for subrow in 0..GRID_Y {
                let y = row_top + subrow;
                while pending.peek().is_some_and(|edge| edge.top == y) {
                    active.push(pending.next().unwrap());
                }
                active.sort_by_key(Edge::cell);
                let mut winding = 0i32;
                let mut start = None;
                for edge in &active {
                    winding += edge.direction;
                    let inside = if even_odd { winding & 1 != 0 } else { winding != 0 };
                    if inside && start.is_none() {
                        start = Some(edge.cell());
                    } else if !inside && let Some(left) = start.take() {
                        addSubspan(&mut cells, left, edge.cell());
                    }
                }
                active.retain(|edge| edge.bottom > y + 1);
                for edge in &mut active {
                    edge.advance(false);
                }
            }
        }
        active.retain(|edge| edge.bottom > row_top + GRID_Y);
        let mut covered = 0;
        let mut cells = cells.into_iter().peekable();
        while cells.peek().is_some_and(|(column, _)| *column < 0) {
            covered += cells.next().unwrap().1.height * 512;
        }
        for column in 0..width {
            let mut area = covered;
            if cells.peek().is_some_and(|(x, _)| *x == column as i64) {
                let cell = cells.next().unwrap().1;
                covered += cell.height * 512;
                area = covered - cell.area;
            }
            let alpha = ((area * 17 + 256) >> 9).clamp(0, 255) as u8;
            mask.data_mut()[row as usize * width as usize + column as usize] = alpha;
        }
    }
    Some(mask)
}
