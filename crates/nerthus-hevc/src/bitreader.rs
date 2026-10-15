//! The bit reader libde265 parses headers with (`bitstream.cc`): up to 64
//! bits held left-aligned, refilled a byte at a time. Reads past the end
//! give zero bits; shifts wrap the way x86 masks them, as the C code's
//! out-of-range shifts do.

pub const MAX_UVLC_LEADING_ZEROS: i32 = 20;
pub const UVLC_ERROR: i32 = -99999;

#[derive(Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    position: usize,
    next_bits: u64,
    next_bits_count: i32,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> BitReader<'a> {
        let mut reader = BitReader { data, position: 0, next_bits: 0, next_bits_count: 0 };
        reader.refill();
        reader
    }

    /// Where `data` points: past the end after a header that read beyond
    /// it (`prepare_for_cabac` then moves forward, as in C).
    pub fn position(&self) -> usize {
        self.position
    }

    fn refill(&mut self) {
        let mut shift = 64 - self.next_bits_count;
        while shift >= 8 && self.position < self.data.len() {
            let byte = self.data[self.position] as u64;
            self.position += 1;
            shift -= 8;
            self.next_bits |= byte.wrapping_shl(shift as u32);
        }
        self.next_bits_count = 64 - shift;
    }

    pub fn get_bits(&mut self, count: i32) -> i32 {
        if self.next_bits_count < count {
            self.refill();
        }
        let value = self.next_bits.wrapping_shr((64 - count) as u32);
        self.next_bits = self.next_bits.wrapping_shl(count as u32);
        self.next_bits_count -= count;
        value as i32
    }

    pub fn skip_bits(&mut self, count: i32) {
        if self.next_bits_count < count {
            self.refill();
        }
        self.next_bits = self.next_bits.wrapping_shl(count as u32);
        self.next_bits_count -= count;
    }

    pub fn skip_to_byte_boundary(&mut self) {
        let skip = self.next_bits_count & 7;
        self.next_bits = self.next_bits.wrapping_shl(skip as u32);
        self.next_bits_count -= skip;
    }

    /// `prepare_for_CABAC`: back to the first whole byte not yet consumed
    /// (forward when more bits were read than there were).
    pub fn prepare_for_cabac(&mut self) {
        self.skip_to_byte_boundary();
        let rewind = (self.next_bits_count / 8) as isize;
        self.position = (self.position as isize - rewind).max(0) as usize;
        self.next_bits = 0;
        self.next_bits_count = 0;
    }

    pub fn get_uvlc(&mut self) -> i32 {
        let mut zeros = 0;
        while self.get_bits(1) == 0 {
            zeros += 1;
            if zeros > MAX_UVLC_LEADING_ZEROS {
                return UVLC_ERROR;
            }
        }
        if zeros == 0 {
            return 0;
        }
        let offset = self.get_bits(zeros);
        offset + (1 << zeros) - 1
    }

    pub fn get_svlc(&mut self) -> i32 {
        let value = self.get_uvlc();
        if value == 0 || value == UVLC_ERROR {
            return value;
        }
        if value & 1 == 0 { -value / 2 } else { (value + 1) / 2 }
    }
}
