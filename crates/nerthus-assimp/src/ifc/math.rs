//! `aiVector2t`, `aiVector3t`, `aiMatrix3x3t` and `aiMatrix4x4t` in
//! doubles (`IfcFloat`), in Assimp's operation order: `Normalize` leaves
//! a zero vector as it is and multiplies by the reciprocal of the length
//! (the 2D one divides), `/=` ignores a zero divisor while `/` multiplies
//! by `1 / f`, `operator[]` past z is x, and `operator<` orders by x, then
//! y, then z. `std::min`/`std::max` of vectors (ProcessHelper's overloads)
//! work per component.

use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Vector3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vector3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Vector3 { x, y, z }
    }

    pub fn square_length(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    pub fn length(self) -> f64 {
        self.square_length().sqrt()
    }

    /// `Normalize`.
    pub fn normalize(self) -> Vector3 {
        if self.length() == 0.0 {
            return self;
        }
        self.divided(self.length())
    }

    /// `operator/=`: unchanged by 0, else times `1 / f`.
    pub fn divided(self, f: f64) -> Vector3 {
        if f == 0.0 {
            return self;
        }
        let inverse = 1.0 / f;
        Vector3::new(self.x * inverse, self.y * inverse, self.z * inverse)
    }

    pub fn set(&mut self, index: usize, value: f64) {
        match index {
            1 => self.y = value,
            2 => self.z = value,
            _ => self.x = value,
        }
    }

    /// `std::min(self, other)`, per component.
    pub fn min(self, other: Vector3) -> Vector3 {
        Vector3::new(std_min(self.x, other.x), std_min(self.y, other.y), std_min(self.z, other.z))
    }

    /// `std::max(self, other)`, per component.
    pub fn max(self, other: Vector3) -> Vector3 {
        Vector3::new(std_max(self.x, other.x), std_max(self.y, other.y), std_max(self.z, other.z))
    }

    /// `MinMaxChooser`'s start for `ArrayBounds`: (1e10, -1e10).
    pub fn bounds_start() -> (Vector3, Vector3) {
        let far = 1e10f32 as f64;
        (Vector3::new(far, far, far), Vector3::new(-far, -far, -far))
    }

    /// The single-precision vector (`static_cast<float>` of each).
    pub fn to_f32(self) -> [f32; 3] {
        [self.x as f32, self.y as f32, self.z as f32]
    }
}

/// `std::min(a, b)`: `b < a ? b : a`.
pub(crate) fn std_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// `std::max(a, b)`: `a < b ? b : a`.
pub(crate) fn std_max(a: f64, b: f64) -> f64 {
    if a < b { b } else { a }
}

/// `ArrayBounds`: `min = std::min(v, min)`, `max = std::max(v, max)`.
pub(crate) fn array_bounds(points: &[Vector3]) -> (Vector3, Vector3) {
    let (mut min, mut max) = Vector3::bounds_start();
    for &point in points {
        min = point.min(min);
        max = point.max(max);
    }
    (min, max)
}

impl Add for Vector3 {
    type Output = Vector3;
    fn add(self, other: Vector3) -> Vector3 {
        Vector3::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl Sub for Vector3 {
    type Output = Vector3;
    fn sub(self, other: Vector3) -> Vector3 {
        Vector3::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl Neg for Vector3 {
    type Output = Vector3;
    fn neg(self) -> Vector3 {
        Vector3::new(-self.x, -self.y, -self.z)
    }
}

impl Mul<f64> for Vector3 {
    type Output = Vector3;
    fn mul(self, f: f64) -> Vector3 {
        Vector3::new(f * self.x, f * self.y, f * self.z)
    }
}

impl Mul<Vector3> for f64 {
    type Output = Vector3;
    fn mul(self, v: Vector3) -> Vector3 {
        Vector3::new(self * v.x, self * v.y, self * v.z)
    }
}

/// `operator*` of two vectors: the dot product.
pub(crate) fn dot(a: Vector3, b: Vector3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// `operator^`: the cross product.
pub(crate) fn cross(a: Vector3, b: Vector3) -> Vector3 {
    Vector3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Vector2 {
    pub x: f64,
    pub y: f64,
}

impl Vector2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Vector2 { x, y }
    }

    pub fn square_length(self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    pub fn length(self) -> f64 {
        self.square_length().sqrt()
    }

}

impl Add for Vector2 {
    type Output = Vector2;
    fn add(self, other: Vector2) -> Vector2 {
        Vector2::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for Vector2 {
    type Output = Vector2;
    fn sub(self, other: Vector2) -> Vector2 {
        Vector2::new(self.x - other.x, self.y - other.y)
    }
}

impl Neg for Vector2 {
    type Output = Vector2;
    fn neg(self) -> Vector2 {
        Vector2::new(-self.x, -self.y)
    }
}

impl Mul<f64> for Vector2 {
    type Output = Vector2;
    fn mul(self, f: f64) -> Vector2 {
        Vector2::new(f * self.x, f * self.y)
    }
}

impl Mul<Vector2> for f64 {
    type Output = Vector2;
    fn mul(self, v: Vector2) -> Vector2 {
        Vector2::new(self * v.x, self * v.y)
    }
}

/// `aiMatrix4x4t<double>`, row-major `a1 … d4`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Matrix4(pub [f64; 16]);

impl Default for Matrix4 {
    fn default() -> Self {
        Matrix4::IDENTITY
    }
}

impl Matrix4 {
    pub const IDENTITY: Matrix4 = Matrix4([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);

    /// From a 3x3 matrix (no translation).
    pub fn from_matrix3(m: &Matrix3) -> Matrix4 {
        let [a1, a2, a3, b1, b2, b3, c1, c2, c3] = m.0;
        Matrix4([a1, a2, a3, 0.0, b1, b2, b3, 0.0, c1, c2, c3, 0.0, 0.0, 0.0, 0.0, 1.0])
    }

    /// `operator*`: `self × m`, each entry `m.x1·self.r1 + …`.
    pub fn product(&self, m: &Matrix4) -> Matrix4 {
        let a = self.0;
        let b = m.0;
        let mut result = [0.0f64; 16];
        for row in 0..4 {
            for column in 0..4 {
                let first = b[column] * a[row * 4];
                let second = b[4 + column] * a[row * 4 + 1];
                let third = b[8 + column] * a[row * 4 + 2];
                let fourth = b[12 + column] * a[row * 4 + 3];
                result[row * 4 + column] = first + second + third + fourth;
            }
        }
        Matrix4(result)
    }

    pub fn determinant(&self) -> f64 {
        let [a1, a2, a3, a4, b1, b2, b3, b4, c1, c2, c3, c4, d1, d2, d3, d4] = self.0;
        a1 * b2 * c3 * d4 - a1 * b2 * c4 * d3 + a1 * b3 * c4 * d2 - a1 * b3 * c2 * d4 + a1 * b4 * c2 * d3 - a1 * b4 * c3 * d2 - a2 * b3 * c4 * d1
            + a2 * b3 * c1 * d4
            - a2 * b4 * c1 * d3
            + a2 * b4 * c3 * d1
            - a2 * b1 * c3 * d4
            + a2 * b1 * c4 * d3
            + a3 * b4 * c1 * d2
            - a3 * b4 * c2 * d1
            + a3 * b1 * c2 * d4
            - a3 * b1 * c4 * d2
            + a3 * b2 * c4 * d1
            - a3 * b2 * c1 * d4
            - a4 * b1 * c2 * d3
            + a4 * b1 * c3 * d2
            - a4 * b2 * c3 * d1
            + a4 * b2 * c1 * d3
            - a4 * b3 * c1 * d2
            + a4 * b3 * c2 * d1
    }

    /// `Inverse`: all NaN when the determinant is 0.
    pub fn inverse(&self) -> Matrix4 {
        let det = self.determinant();
        if det == 0.0 {
            return Matrix4([f64::NAN; 16]);
        }
        let invdet = 1.0 / det;
        let [a1, a2, a3, a4, b1, b2, b3, b4, c1, c2, c3, c4, d1, d2, d3, d4] = self.0;
        Matrix4([
            invdet * (b2 * (c3 * d4 - c4 * d3) + b3 * (c4 * d2 - c2 * d4) + b4 * (c2 * d3 - c3 * d2)),
            -invdet * (a2 * (c3 * d4 - c4 * d3) + a3 * (c4 * d2 - c2 * d4) + a4 * (c2 * d3 - c3 * d2)),
            invdet * (a2 * (b3 * d4 - b4 * d3) + a3 * (b4 * d2 - b2 * d4) + a4 * (b2 * d3 - b3 * d2)),
            -invdet * (a2 * (b3 * c4 - b4 * c3) + a3 * (b4 * c2 - b2 * c4) + a4 * (b2 * c3 - b3 * c2)),
            -invdet * (b1 * (c3 * d4 - c4 * d3) + b3 * (c4 * d1 - c1 * d4) + b4 * (c1 * d3 - c3 * d1)),
            invdet * (a1 * (c3 * d4 - c4 * d3) + a3 * (c4 * d1 - c1 * d4) + a4 * (c1 * d3 - c3 * d1)),
            -invdet * (a1 * (b3 * d4 - b4 * d3) + a3 * (b4 * d1 - b1 * d4) + a4 * (b1 * d3 - b3 * d1)),
            invdet * (a1 * (b3 * c4 - b4 * c3) + a3 * (b4 * c1 - b1 * c4) + a4 * (b1 * c3 - b3 * c1)),
            invdet * (b1 * (c2 * d4 - c4 * d2) + b2 * (c4 * d1 - c1 * d4) + b4 * (c1 * d2 - c2 * d1)),
            -invdet * (a1 * (c2 * d4 - c4 * d2) + a2 * (c4 * d1 - c1 * d4) + a4 * (c1 * d2 - c2 * d1)),
            invdet * (a1 * (b2 * d4 - b4 * d2) + a2 * (b4 * d1 - b1 * d4) + a4 * (b1 * d2 - b2 * d1)),
            -invdet * (a1 * (b2 * c4 - b4 * c2) + a2 * (b4 * c1 - b1 * c4) + a4 * (b1 * c2 - b2 * c1)),
            -invdet * (b1 * (c2 * d3 - c3 * d2) + b2 * (c3 * d1 - c1 * d3) + b3 * (c1 * d2 - c2 * d1)),
            invdet * (a1 * (c2 * d3 - c3 * d2) + a2 * (c3 * d1 - c1 * d3) + a3 * (c1 * d2 - c2 * d1)),
            -invdet * (a1 * (b2 * d3 - b3 * d2) + a2 * (b3 * d1 - b1 * d3) + a3 * (b1 * d2 - b2 * d1)),
            invdet * (a1 * (b2 * c3 - b3 * c2) + a2 * (b3 * c1 - b1 * c3) + a3 * (b1 * c2 - b2 * c1)),
        ])
    }

    /// `Translation`.
    pub fn translation(v: Vector3) -> Matrix4 {
        let mut out = Matrix4::IDENTITY;
        out.0[3] = v.x;
        out.0[7] = v.y;
        out.0[11] = v.z;
        out
    }

    /// `Scaling`.
    pub fn scaling(v: Vector3) -> Matrix4 {
        let mut out = Matrix4::IDENTITY;
        out.0[0] = v.x;
        out.0[5] = v.y;
        out.0[10] = v.z;
        out
    }

    /// `Rotation(a, axis)`: about an axis through the origin.
    pub fn rotation(a: f64, axis: Vector3) -> Matrix4 {
        let c = a.cos();
        let s = a.sin();
        let t = 1.0 - c;
        let (x, y, z) = (axis.x, axis.y, axis.z);
        Matrix4([
            t * x * x + c,
            t * x * y - s * z,
            t * x * z + s * y,
            0.0,
            t * x * y + s * z,
            t * y * y + c,
            t * y * z - s * x,
            0.0,
            t * x * z - s * y,
            t * y * z + s * x,
            t * z * z + c,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    /// `matrix * vector`, translation included.
    pub fn transform(&self, v: Vector3) -> Vector3 {
        let m = self.0;
        Vector3::new(
            m[0] * v.x + m[1] * v.y + m[2] * v.z + m[3],
            m[4] * v.x + m[5] * v.y + m[6] * v.z + m[7],
            m[8] * v.x + m[9] * v.y + m[10] * v.z + m[11],
        )
    }

    /// `IfcMatrix3(self)`: the upper-left 3x3.
    pub fn matrix3(&self) -> Matrix3 {
        let m = self.0;
        Matrix3([m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]])
    }

}

/// `aiMatrix3x3t<double>`, row-major `a1 … c3`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Matrix3(pub [f64; 9]);

impl Default for Matrix3 {
    fn default() -> Self {
        Matrix3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
    }
}

impl Matrix3 {
    /// `matrix * vector`.
    pub fn transform(&self, v: Vector3) -> Vector3 {
        let m = self.0;
        Vector3::new(m[0] * v.x + m[1] * v.y + m[2] * v.z, m[3] * v.x + m[4] * v.y + m[5] * v.z, m[6] * v.x + m[7] * v.y + m[8] * v.z)
    }

    pub fn determinant(&self) -> f64 {
        let [a1, a2, a3, b1, b2, b3, c1, c2, c3] = self.0;
        a1 * b2 * c3 - a1 * b3 * c2 + a2 * b3 * c1 - a2 * b1 * c3 + a3 * b1 * c2 - a3 * b2 * c1
    }

    /// `Inverse`: all NaN when the determinant is 0.
    pub fn inverse(&self) -> Matrix3 {
        let det = self.determinant();
        if det == 0.0 {
            return Matrix3([f64::NAN; 9]);
        }
        let invdet = 1.0 / det;
        let [a1, a2, a3, b1, b2, b3, c1, c2, c3] = self.0;
        Matrix3([
            invdet * (b2 * c3 - b3 * c2),
            -invdet * (a2 * c3 - a3 * c2),
            invdet * (a2 * b3 - a3 * b2),
            -invdet * (b1 * c3 - b3 * c1),
            invdet * (a1 * c3 - a3 * c1),
            -invdet * (a1 * b3 - a3 * b1),
            invdet * (b1 * c2 - b2 * c1),
            -invdet * (a1 * c2 - a2 * c1),
            invdet * (a1 * b2 - a2 * b1),
        ])
    }
}
