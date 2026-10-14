//! Open3DGC's `BinaryStream` reads: little-endian 32-bit words, or in the
//! ASCII form seven bits a byte (variable-length integers in 7- and
//! 6-bit groups). Positions are 32-bit (`unsigned long` on Windows).
//!
//! The stream is a copy of the compressed bytes in a buffer of at least
//! 4096 bytes, so reads past its end see whatever that buffer held. A
//! valid stream's last arithmetic-coded block reads up to three bytes
//! past it, which change nothing it decodes: those few bytes are taken as
//! zero here, and reads further on as undefined.

use super::codec::{uint_to_int, undefined};
use crate::scene::{self, ImportError};

/// `O3DGCStreamType` past its start code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StreamType {
    Ascii,
    Binary,
}

/// `O3DGC_BINARY_STREAM_*`.
const SYMBOL0_MAX: u32 = 127;
const SYMBOL0_BITS: u32 = 7;
const SYMBOL1_BITS: u32 = 6;
const SYMBOLS_PER_UINT32: u32 = 5;

/// How far past the stream's end reads see zeros.
const READ_AHEAD: usize = 4;

/// The stream's byte at `position`: zero just past its end, undefined
/// further on.
pub(super) fn stream_byte(bytes: &[u8], position: usize) -> Result<u8, ImportError> {
    if let Some(&byte) = bytes.get(position) {
        return Ok(byte);
    }
    if position < bytes.len() + READ_AHEAD {
        return Ok(0);
    }
    undefined("an Open3DGC stream read past its end")
}

pub(crate) struct Stream<'a> {
    bytes: &'a [u8],
}

impl<'a> Stream<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Stream { bytes }
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    fn byte(&self, position: &mut u32) -> Result<u8, ImportError> {
        let byte = stream_byte(self.bytes, *position as usize)?;
        *position = position.wrapping_add(1);
        Ok(byte)
    }

    /// `ReadUInt32Bin`, which asserts the word starts more than four bytes
    /// before the stream's end (the size is 32-bit, so a shorter stream
    /// passes).
    pub fn read_u32_binary(&self, position: &mut u32) -> Result<u32, ImportError> {
        let size = self.bytes.len() as u32;
        scene::ai_assert(*position < size.wrapping_sub(4), "position < m_stream.GetSize() - 4")?;
        let mut value = 0u32;
        for shift in [0u32, 8, 16, 24] {
            value = value.wrapping_add((self.byte(position)? as u32) << shift);
        }
        Ok(value)
    }

    /// `ReadUInt32ASCII`: five 7-bit groups, the last shifted out of range
    /// (asserted to start more than five bytes before the stream's end).
    pub fn read_u32_ascii(&self, position: &mut u32) -> Result<u32, ImportError> {
        let size = self.bytes.len() as u32;
        let fits = *position < size.wrapping_sub(SYMBOLS_PER_UINT32);
        scene::ai_assert(fits, "position < m_stream.GetSize() - O3DGC_BINARY_STREAM_NUM_SYMBOLS_UINT32")?;
        let mut value = 0u32;
        let mut shift = 0u32;
        for _ in 0..SYMBOLS_PER_UINT32 {
            value = value.wrapping_add((self.byte(position)? as u32).wrapping_shl(shift));
            shift += SYMBOL0_BITS;
        }
        Ok(value)
    }

    pub fn read_u32(&self, position: &mut u32, stream_type: StreamType) -> Result<u32, ImportError> {
        match stream_type {
            StreamType::Ascii => self.read_u32_ascii(position),
            StreamType::Binary => self.read_u32_binary(position),
        }
    }

    pub fn read_f32(&self, position: &mut u32, stream_type: StreamType) -> Result<f32, ImportError> {
        Ok(f32::from_bits(self.read_u32(position, stream_type)?))
    }

    /// `ReadUChar` (`ReadUChar8Bin` and `ReadUCharASCII` alike).
    pub fn read_uchar(&self, position: &mut u32) -> Result<u8, ImportError> {
        self.byte(position)
    }

    /// `ReadUIntASCII`: a byte, continued in 6-bit groups after 127 (their
    /// shifts in 32-bit `long`s, masked as x86 does).
    pub fn read_uint_ascii(&self, position: &mut u32) -> Result<u32, ImportError> {
        let mut value = self.byte(position)? as u32;
        if value == SYMBOL0_MAX {
            let mut shift = 0u32;
            loop {
                let group = self.byte(position)? as i32;
                value = value.wrapping_add(((group >> 1) as u32).wrapping_shl(shift));
                shift = shift.wrapping_add(SYMBOL1_BITS);
                if group & 1 == 0 {
                    break;
                }
            }
        }
        Ok(value)
    }

    /// `ReadIntASCII`.
    pub fn read_int_ascii(&self, position: &mut u32) -> Result<i32, ImportError> {
        Ok(uint_to_int(self.read_uint_ascii(position)?))
    }
}
