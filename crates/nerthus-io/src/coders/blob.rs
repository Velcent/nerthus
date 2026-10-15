//! A file as ImageMagick's readers see it (`BlobInfo`): reads that run past
//! the end come up short and set end of file, which a seek back into the
//! file clears; the number readers give zero when cut short.

pub(crate) struct Blob<'a> {
    bytes: &'a [u8],
    offset: usize,
    /// `EOFBlob`: a read went past the end.
    pub eof: bool,
}

impl<'a> Blob<'a> {
    pub fn new(bytes: &'a [u8]) -> Blob<'a> {
        Blob { bytes, offset: 0, eof: false }
    }

    /// `GetBlobSize`.
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// `TellBlob`.
    pub fn tell(&self) -> u64 {
        self.offset as u64
    }

    /// `ReadBlob`: up to `length` bytes (none asked for reads nothing).
    pub fn read(&mut self, length: usize) -> &'a [u8] {
        if length == 0 {
            return &[];
        }
        if self.offset >= self.bytes.len() {
            self.eof = true;
            return &[];
        }
        let end = self.offset.saturating_add(length).min(self.bytes.len());
        let read = &self.bytes[self.offset..end];
        self.offset = end;
        if read.len() != length {
            self.eof = true;
        }
        read
    }

    /// `ReadBlobByte`: none at the end.
    pub fn byte(&mut self) -> Option<u8> {
        if self.offset >= self.bytes.len() {
            self.eof = true;
            return None;
        }
        let byte = self.bytes[self.offset];
        self.offset += 1;
        Some(byte)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let bytes = self.read(N);
        if bytes.len() != N {
            return None;
        }
        let mut array = [0u8; N];
        array.copy_from_slice(bytes);
        Some(array)
    }

    /// `ReadBlobLSBShort`.
    pub fn lsb_u16(&mut self) -> u16 {
        self.array::<2>().map_or(0, u16::from_le_bytes)
    }

    /// `ReadBlobLSBLong`.
    pub fn lsb_u32(&mut self) -> u32 {
        self.array::<4>().map_or(0, u32::from_le_bytes)
    }

    /// `ReadBlobMSBShort`.
    pub fn msb_u16(&mut self) -> u16 {
        self.array::<2>().map_or(0, u16::from_be_bytes)
    }

    /// `ReadBlobMSBLong`.
    pub fn msb_u32(&mut self) -> u32 {
        self.array::<4>().map_or(0, u32::from_be_bytes)
    }

    /// `ReadBlobString`: a line, its newline (or one carriage return)
    /// dropped; `None` at the end of the file. Files up to
    /// `MagickMaxBufferExtent` are read with `fgets` (4095 bytes at most, cut
    /// at a NUL); bigger ones are memory mapped and read a byte at a time
    /// (4094 bytes at most, and a last line that strips to nothing is none).
    pub fn string(&mut self) -> Option<Vec<u8>> {
        const MAX_BUFFER_EXTENT: usize = 524288;
        let mapped = self.bytes.len() > MAX_BUFFER_EXTENT;
        let limit = if mapped { 4094 } else { 4095 };
        let mut line = Vec::new();
        while line.len() < limit {
            let Some(byte) = self.byte() else {
                break;
            };
            line.push(byte);
            if byte == b'\n' {
                break;
            }
        }
        if !mapped {
            if line.is_empty() {
                return None;
            }
            if let Some(nul) = line.iter().position(|&byte| byte == 0) {
                line.truncate(nul);
            }
        }
        if matches!(line.last(), Some(b'\r' | b'\n')) {
            line.pop();
        }
        if mapped {
            if let Some(nul) = line.iter().position(|&byte| byte == 0) {
                line.truncate(nul);
            }
            if line.is_empty() && self.eof {
                return None;
            }
        }
        Some(line)
    }

    /// `SeekBlob(SEEK_SET)`: `None` for a negative offset.
    pub fn seek(&mut self, offset: i64) -> Option<u64> {
        if offset < 0 {
            return None;
        }
        self.offset = usize::try_from(offset).unwrap_or(usize::MAX);
        if self.offset < self.bytes.len() {
            self.eof = false;
        }
        Some(offset as u64)
    }
}
