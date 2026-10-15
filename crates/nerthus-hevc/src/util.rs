//! libde265's small helpers (`util.h`).

pub fn clip3(low: i32, high: i32, value: i32) -> i32 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

pub fn ceil_div(num: i32, denom: i32) -> i32 {
    (num + denom - 1) / denom
}

pub fn ceil_log2(value: i32) -> i32 {
    let mut n = 0;
    while value > (1 << n) {
        n += 1;
    }
    n
}
