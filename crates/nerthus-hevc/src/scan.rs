//! Coefficient scan orders (`scan.cc`): diagonal, horizontal and vertical
//! scans of 2x2 to 32x32 blocks, and where each position falls in the
//! 4x4 sub-block scan.

use std::sync::OnceLock;

#[derive(Clone, Copy, Default, Debug)]
pub struct Position {
    pub x: u8,
    pub y: u8,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct ScanPosition {
    pub sub_block: u8,
    pub scan_pos: u8,
}

struct Tables {
    /// [scan index][log2 size] → positions.
    orders: [[Vec<Position>; 6]; 3],
    /// [scan index][log2 size] → per (x,y), its sub-block and position.
    positions: [[Vec<ScanPosition>; 6]; 3],
}

fn diagonal(size: usize) -> Vec<Position> {
    let mut scan = Vec::with_capacity(size * size);
    let mut x: i32 = 0;
    let mut y: i32 = 0;
    loop {
        while y >= 0 {
            if (x as usize) < size && (y as usize) < size {
                scan.push(Position { x: x as u8, y: y as u8 });
            }
            y -= 1;
            x += 1;
        }
        y = x;
        x = 0;
        if scan.len() >= size * size {
            break;
        }
    }
    scan
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut orders: [[Vec<Position>; 6]; 3] = Default::default();
        for scan in orders.iter_mut() {
            scan[0] = vec![Position::default()];
        }
        for log2 in 1..=5 {
            let size = 1usize << log2;
            orders[0][log2] = diagonal(size);
            let mut horizontal = Vec::with_capacity(size * size);
            let mut vertical = Vec::with_capacity(size * size);
            for a in 0..size {
                for b in 0..size {
                    horizontal.push(Position { x: b as u8, y: a as u8 });
                    vertical.push(Position { x: a as u8, y: b as u8 });
                }
            }
            orders[1][log2] = horizontal;
            orders[2][log2] = vertical;
        }
        let mut positions: [[Vec<ScanPosition>; 6]; 3] = Default::default();
        for log2 in 2..=5usize {
            let size = 1usize << log2;
            for scan_index in 0..3 {
                let sub = &orders[scan_index][log2 - 2];
                let pos = &orders[scan_index][2];
                let mut table = vec![ScanPosition::default(); size * size];
                for y in 0..size {
                    for x in 0..size {
                        let mut last_scan_pos = 16i32;
                        let mut last_sub_block = (1i32 << (log2 - 2)) * (1i32 << (log2 - 2)) - 1;
                        loop {
                            if last_scan_pos == 0 {
                                last_scan_pos = 16;
                                last_sub_block -= 1;
                            }
                            last_scan_pos -= 1;
                            let s = sub[last_sub_block as usize];
                            let p = pos[last_scan_pos as usize];
                            let x_c = ((s.x as usize) << 2) + p.x as usize;
                            let y_c = ((s.y as usize) << 2) + p.y as usize;
                            if x_c == x && y_c == y {
                                break;
                            }
                        }
                        table[y * size + x] = ScanPosition { sub_block: last_sub_block as u8, scan_pos: last_scan_pos as u8 };
                    }
                }
                positions[scan_index][log2] = table;
            }
        }
        Tables { orders, positions }
    })
}

/// `get_scan_order`: 0 diagonal, 1 horizontal, 2 vertical.
pub fn scan_order(log2_block_size: usize, scan_index: usize) -> &'static [Position] {
    &tables().orders[scan_index][log2_block_size]
}

pub fn scan_position(x: usize, y: usize, scan_index: usize, log2_block_size: usize) -> ScanPosition {
    tables().positions[scan_index][log2_block_size][(y << log2_block_size) + x]
}
