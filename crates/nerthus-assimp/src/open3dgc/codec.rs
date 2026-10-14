//! Open3DGC's arithmetic decoder (Amir Said's FastAC, `Arithmetic_Codec`)
//! and its static and adaptive bit and data models, in 32-bit unsigned
//! arithmetic. The decoder reads on from wherever its block starts, into
//! the blocks after it and past the stream's end as the stream's reads
//! do. A buffer or model of an invalid size makes the codec print an
//! error and exit the process.

use super::stream::stream_byte;
use crate::scene::{ImportError, fail};

/// `AC__MinLength`, `AC__MaxLength`.
const MIN_LENGTH: u32 = 0x0100_0000;
const MAX_LENGTH: u32 = 0xFFFF_FFFF;

/// `BM__LengthShift`, `BM__MaxCount`, `DM__LengthShift`, `DM__MaxCount`.
const BIT_LENGTH_SHIFT: u32 = 13;
const BIT_MAX_COUNT: u32 = 1 << BIT_LENGTH_SHIFT;
const DATA_LENGTH_SHIFT: u32 = 15;
const DATA_MAX_COUNT: u32 = 1 << DATA_LENGTH_SHIFT;

pub(super) fn undefined<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("undefined behaviour: {what}"))
}

/// `AC_Error`: a message, then `exit(1)`.
pub(super) fn codec_exit<T>(what: &str) -> Result<T, ImportError> {
    fail(format!("process exit: Open3DGC arithmetic coding error ({what})"))
}

/// `Static_Bit_Model`: an even split.
pub(super) struct StaticBitModel {
    bit_0_prob: u32,
}

impl StaticBitModel {
    pub fn new() -> Self {
        StaticBitModel { bit_0_prob: 1 << (BIT_LENGTH_SHIFT - 1) }
    }
}

/// `Adaptive_Bit_Model`.
pub(super) struct AdaptiveBitModel {
    update_cycle: u32,
    bits_until_update: u32,
    bit_0_prob: u32,
    bit_0_count: u32,
    bit_count: u32,
}

impl AdaptiveBitModel {
    pub fn new() -> Self {
        AdaptiveBitModel { update_cycle: 4, bits_until_update: 4, bit_0_prob: 1 << (BIT_LENGTH_SHIFT - 1), bit_0_count: 1, bit_count: 2 }
    }

    /// `update`: counts halved past the limit, a new probability, and
    /// updates further apart.
    fn update(&mut self) {
        self.bit_count = self.bit_count.wrapping_add(self.update_cycle);
        if self.bit_count > BIT_MAX_COUNT {
            self.bit_count = (self.bit_count + 1) >> 1;
            self.bit_0_count = (self.bit_0_count + 1) >> 1;
            if self.bit_0_count == self.bit_count {
                self.bit_count += 1;
            }
        }
        let scale = 0x8000_0000u32 / self.bit_count;
        self.bit_0_prob = self.bit_0_count.wrapping_mul(scale) >> (31 - BIT_LENGTH_SHIFT);
        self.update_cycle = (5 * self.update_cycle) >> 2;
        if self.update_cycle > 64 {
            self.update_cycle = 64;
        }
        self.bits_until_update = self.update_cycle;
    }
}

/// `Adaptive_Data_Model`: symbol counts, the cumulative distribution, and
/// a decoding table for alphabets over 16 symbols.
pub(super) struct AdaptiveDataModel {
    distribution: Vec<u32>,
    symbol_count: Vec<u32>,
    decoder_table: Option<Vec<u32>>,
    total_count: u32,
    update_cycle: u32,
    symbols_until_update: u32,
    data_symbols: u32,
    last_symbol: u32,
    table_shift: u32,
}

impl AdaptiveDataModel {
    /// `Adaptive_Data_Model(n)` (`set_alphabet`): 2 to 2048 symbols, else
    /// the codec exits.
    pub fn new(symbols: u32) -> Result<Self, ImportError> {
        if !(2..=1 << 11).contains(&symbols) {
            return codec_exit("invalid number of data symbols");
        }
        let mut table_shift = 0;
        let mut decoder_table = None;
        if symbols > 16 {
            let mut table_bits = 3;
            while symbols > (1u32 << (table_bits + 2)) {
                table_bits += 1;
            }
            let table_size = 1usize << table_bits;
            table_shift = DATA_LENGTH_SHIFT - table_bits;
            decoder_table = Some(vec![0u32; table_size + 2]);
        }
        let mut model = AdaptiveDataModel {
            distribution: vec![0; symbols as usize],
            symbol_count: vec![1; symbols as usize],
            decoder_table,
            total_count: 0,
            update_cycle: symbols,
            symbols_until_update: 0,
            data_symbols: symbols,
            last_symbol: symbols - 1,
            table_shift,
        };
        model.update();
        model.update_cycle = (symbols + 6) >> 1;
        model.symbols_until_update = model.update_cycle;
        Ok(model)
    }

    /// `update(false)`: counts halved past the limit, the distribution and
    /// table rebuilt, and updates further apart.
    fn update(&mut self) {
        self.total_count = self.total_count.wrapping_add(self.update_cycle);
        if self.total_count > DATA_MAX_COUNT {
            self.total_count = 0;
            for count in &mut self.symbol_count {
                *count = (*count + 1) >> 1;
                self.total_count = self.total_count.wrapping_add(*count);
            }
        }
        let scale = 0x8000_0000u32 / self.total_count.max(1);
        let mut sum = 0u32;
        let mut filled = 0usize;
        for symbol in 0..self.data_symbols as usize {
            self.distribution[symbol] = scale.wrapping_mul(sum) >> (31 - DATA_LENGTH_SHIFT);
            sum = sum.wrapping_add(self.symbol_count[symbol]);
            if let Some(table) = self.decoder_table.as_mut() {
                let reach = (self.distribution[symbol] >> self.table_shift) as usize;
                while filled < reach {
                    filled += 1;
                    table[filled] = (symbol as u32).wrapping_sub(1);
                }
            }
        }
        if let Some(table) = self.decoder_table.as_mut() {
            table[0] = 0;
            let table_size = table.len() - 2;
            while filled <= table_size {
                filled += 1;
                table[filled] = self.data_symbols - 1;
            }
        }
        self.update_cycle = (5 * self.update_cycle) >> 2;
        let max_cycle = (self.data_symbols + 6) << 3;
        if self.update_cycle > max_cycle {
            self.update_cycle = max_cycle;
        }
        self.symbols_until_update = self.update_cycle;
    }
}

/// `Arithmetic_Codec` as a decoder over a stream from a byte offset.
pub(super) struct Decoder<'a> {
    stream: &'a [u8],
    /// `ac_pointer`: the last byte read.
    pointer: usize,
    value: u32,
    length: u32,
}

impl<'a> Decoder<'a> {
    /// `set_buffer` and `start_decoder`: the first four bytes are the
    /// code value. A block of no bytes makes the codec exit.
    pub fn start(stream: &'a [u8], position: usize, size: u32) -> Result<Self, ImportError> {
        if size == 0 {
            return codec_exit("invalid codec buffer size");
        }
        let mut value = 0u32;
        for offset in 0..4 {
            value = (value << 8) | stream_byte(stream, position + offset)? as u32;
        }
        Ok(Decoder { stream, pointer: position + 3, value, length: MAX_LENGTH })
    }

    /// `renorm_dec_interval`.
    fn renormalize(&mut self) -> Result<(), ImportError> {
        loop {
            self.pointer += 1;
            let byte = stream_byte(self.stream, self.pointer)?;
            self.value = (self.value << 8) | byte as u32;
            self.length <<= 8;
            if self.length >= MIN_LENGTH {
                return Ok(());
            }
        }
    }

    /// `decode(Static_Bit_Model&)`.
    pub fn decode_static_bit(&mut self, model: &StaticBitModel) -> Result<u32, ImportError> {
        let split = model.bit_0_prob.wrapping_mul(self.length >> BIT_LENGTH_SHIFT);
        let bit = (self.value >= split) as u32;
        if bit == 0 {
            self.length = split;
        } else {
            self.value = self.value.wrapping_sub(split);
            self.length = self.length.wrapping_sub(split);
        }
        if self.length < MIN_LENGTH {
            self.renormalize()?;
        }
        Ok(bit)
    }

    /// `decode(Adaptive_Bit_Model&)`.
    pub fn decode_adaptive_bit(&mut self, model: &mut AdaptiveBitModel) -> Result<u32, ImportError> {
        let split = model.bit_0_prob.wrapping_mul(self.length >> BIT_LENGTH_SHIFT);
        let bit = (self.value >= split) as u32;
        if bit == 0 {
            self.length = split;
            model.bit_0_count = model.bit_0_count.wrapping_add(1);
        } else {
            self.value = self.value.wrapping_sub(split);
            self.length = self.length.wrapping_sub(split);
        }
        if self.length < MIN_LENGTH {
            self.renormalize()?;
        }
        model.bits_until_update = model.bits_until_update.wrapping_sub(1);
        if model.bits_until_update == 0 {
            model.update();
        }
        Ok(bit)
    }

    /// `decode(Adaptive_Data_Model&)`: a table look-up and bisection, or
    /// bisection alone for small alphabets.
    pub fn decode_data(&mut self, model: &mut AdaptiveDataModel) -> Result<u32, ImportError> {
        let mut y = self.length;
        let symbol;
        let x;
        if let Some(table) = model.decoder_table.as_ref() {
            self.length >>= DATA_LENGTH_SHIFT;
            let scaled = self.value / self.length.max(1);
            let entry = (scaled >> model.table_shift) as usize;
            let (Some(&first), Some(&next)) = (table.get(entry), table.get(entry + 1)) else {
                return undefined("an Open3DGC decoding table read past its end");
            };
            let mut low = first;
            let mut high = next.wrapping_add(1);
            while high > low.wrapping_add(1) {
                let middle = (low.wrapping_add(high)) >> 1;
                let Some(&bound) = model.distribution.get(middle as usize) else {
                    return undefined("an Open3DGC distribution read past its end");
                };
                if bound > scaled {
                    high = middle;
                } else {
                    low = middle;
                }
            }
            let Some(&low_bound) = model.distribution.get(low as usize) else {
                return undefined("an Open3DGC distribution read past its end");
            };
            x = low_bound.wrapping_mul(self.length);
            if low != model.last_symbol {
                let Some(&high_bound) = model.distribution.get(low as usize + 1) else {
                    return undefined("an Open3DGC distribution read past its end");
                };
                y = high_bound.wrapping_mul(self.length);
            }
            symbol = low;
        } else {
            let mut low = 0u32;
            let mut low_product = 0u32;
            self.length >>= DATA_LENGTH_SHIFT;
            let mut high = model.data_symbols;
            let mut middle = high >> 1;
            loop {
                let product = self.length.wrapping_mul(model.distribution[middle as usize]);
                if product > self.value {
                    high = middle;
                    y = product;
                } else {
                    low = middle;
                    low_product = product;
                }
                middle = (low + high) >> 1;
                if middle == low {
                    break;
                }
            }
            x = low_product;
            symbol = low;
        }
        self.value = self.value.wrapping_sub(x);
        self.length = y.wrapping_sub(x);
        if self.length < MIN_LENGTH {
            self.renormalize()?;
        }
        let Some(count) = model.symbol_count.get_mut(symbol as usize) else {
            return undefined("an Open3DGC symbol count written past its end");
        };
        *count = count.wrapping_add(1);
        model.symbols_until_update = model.symbols_until_update.wrapping_sub(1);
        if model.symbols_until_update == 0 {
            model.update();
        }
        Ok(symbol)
    }

    /// `ExpGolombDecode`: a unary prefix (adaptive), then `k` bits
    /// (static), in `int` arithmetic with x86 shifts.
    pub fn exp_golomb(&mut self, mut k: i32, static_model: &StaticBitModel, adaptive_model: &mut AdaptiveBitModel) -> Result<u32, ImportError> {
        let mut symbol = 0i32;
        let mut binary_symbol = 0i32;
        loop {
            let bit = self.decode_adaptive_bit(adaptive_model)?;
            if bit != 1 {
                break;
            }
            symbol = symbol.wrapping_add(1i32.wrapping_shl(k as u32));
            k = k.wrapping_add(1);
        }
        while k != 0 {
            k = k.wrapping_sub(1);
            if self.decode_static_bit(static_model)? == 1 {
                binary_symbol |= 1i32.wrapping_shl(k as u32);
            }
        }
        Ok(symbol.wrapping_add(binary_symbol) as u32)
    }
}

/// `UIntToInt` in 32-bit `long`s.
pub(super) fn uint_to_int(value: u32) -> i32 {
    if value & 1 == 1 {
        (value.wrapping_add(1) >> 1).wrapping_neg() as i32
    } else {
        (value >> 1) as i32
    }
}

/// `DecodeUIntACEGC`: a symbol, extended by Exp-Golomb at `m`.
pub(super) fn decode_uint_egc(
    decoder: &mut Decoder,
    values: &mut AdaptiveDataModel,
    static_model: &StaticBitModel,
    adaptive_model: &mut AdaptiveBitModel,
    exp_k: u32,
    m: u32,
) -> Result<u32, ImportError> {
    let mut value = decoder.decode_data(values)?;
    if value == m {
        let extension = decoder.exp_golomb(exp_k as i32, static_model, adaptive_model)?;
        value = value.wrapping_add(extension);
    }
    Ok(value)
}

/// `DecodeIntACEGC`.
pub(super) fn decode_int_egc(
    decoder: &mut Decoder,
    values: &mut AdaptiveDataModel,
    static_model: &StaticBitModel,
    adaptive_model: &mut AdaptiveBitModel,
    exp_k: u32,
    m: u32,
) -> Result<i32, ImportError> {
    let value = decode_uint_egc(decoder, values, static_model, adaptive_model, exp_k, m)?;
    Ok(uint_to_int(value))
}
