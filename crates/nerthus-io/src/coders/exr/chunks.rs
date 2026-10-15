//! Chunks as OpenEXRCore 3.4.9 finds them (`chunk.c`): the offset table,
//! rebuilt from the chunks' own headers when it points outside the file,
//! each chunk's header checked against the chunk asked for, and the
//! chunk's size and place in the image.

use anyhow::{anyhow, bail, Result};

use super::header::{sampled_height, sampled_width, Headers, Part, Storage, COMPRESSION_NONE, LINE_ORDER_DECREASING_Y, TILE_RIPMAP_LEVELS};

/// A chunk to decode (`exr_chunk_info_t`).
#[derive(Clone, Debug)]
pub(super) struct ChunkInfo {
    pub start_x: i32,
    pub start_y: i32,
    pub width: i32,
    pub height: i32,
    pub data_offset: u64,
    pub packed_size: u64,
    pub unpacked_size: u64,
}

/// The file and its chunk tables, read when first needed.
pub(super) struct Chunks<'a> {
    pub data: &'a [u8],
    pub headers: Headers,
    tables: Vec<Option<std::result::Result<Vec<u64>, String>>>,
}

fn read_i32(data: &[u8], offset: u64) -> Option<i32> {
    let start = usize::try_from(offset).ok()?;
    let bytes = data.get(start..start.checked_add(4)?)?;
    Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(data: &[u8], offset: u64) -> Option<u64> {
    let start = usize::try_from(offset).ok()?;
    let bytes = data.get(start..start.checked_add(8)?)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

/// A chunk's header (`priv_chunk_leader`).
struct Leader {
    scanline_y: i32,
    tile: [i32; 4],
    next_offset: u64,
}

impl<'a> Chunks<'a> {
    pub fn new(data: &'a [u8], headers: Headers) -> Self {
        let tables = vec![None; headers.parts.len()];
        Self { data, headers, tables }
    }

    fn file_size(&self) -> u64 {
        self.data.len() as u64
    }

    /// `extract_chunk_leader`.
    fn extract_leader(&self, part: &Part, part_number: usize, offset: u64) -> Result<Leader> {
        let multipart = self.headers.multipart;
        let mut position = offset;
        let mut next = || -> Result<i32> {
            let value = read_i32(self.data, position).ok_or_else(|| anyhow!("read past end of file"))?;
            position += 4;
            Ok(value)
        };
        if multipart && next()? != part_number as i32 {
            bail!("Invalid part number reconstructing chunk table");
        }
        let mut leader = Leader { scanline_y: 0, tile: [0; 4], next_offset: 0 };
        if part.storage.is_scanline() {
            leader.scanline_y = next()?;
        } else {
            for value in leader.tile.iter_mut() {
                *value = next()?;
            }
        }
        let max_value = self.file_size() as i64;
        let packed_size = if part.storage.is_deep() {
            let mut deep = [0i64; 3];
            for value in deep.iter_mut() {
                let low = next()? as u32 as u64;
                let high = next()? as u32 as u64;
                *value = (high << 32 | low) as i64;
            }
            if deep[0] < 0 || (deep[0] == 0 && (deep[1] != 0 || deep[2] != 0)) {
                bail!("Invalid chunk size reconstructing chunk table");
            }
            if deep[1] < 0 || deep[1] > max_value || (deep[1] == 0 && deep[2] != 0) {
                bail!("Invalid chunk size reconstructing chunk table");
            }
            deep[0] as u64 + deep[1] as u64
        } else {
            let size = next()?;
            if size < 0 || size as i64 > max_value {
                bail!("Invalid chunk size reconstructing chunk table");
            }
            size as u64
        };
        leader.next_offset = position + packed_size;
        Ok(leader)
    }

    /// `read_and_validate_chunk_leader`: the chunk index the header at
    /// `offset` claims, and where the next chunk starts (known once the
    /// header could be read, even when it names the wrong chunk).
    fn validate_leader(&self, part: &Part, part_number: usize, offset: u64, index: &mut i32) -> (Result<()>, Option<u64>) {
        let leader = match self.extract_leader(part, part_number, offset) {
            Ok(leader) => leader,
            Err(error) => return (Err(error), None),
        };
        let next = Some(leader.next_offset);
        if part.storage.is_scanline() {
            let window = part.data_window();
            let chunk = (leader.scanline_y as i64 - window.min_y as i64) / part.lines_per_chunk as i64;
            let valid = *index as i64 == chunk && chunk >= 0 && chunk < part.chunk_count as i64;
            *index = chunk as i32;
            if !valid {
                return (Err(anyhow!("Invalid chunk index")), next);
            }
        } else {
            let [x, y, level_x, level_y] = leader.tile;
            match tile_chunk_index(part, x, y, level_x, level_y) {
                Ok(found) => *index = found,
                Err(error) => {
                    *index = 0;
                    return (Err(error), next);
                }
            }
        }
        (Ok(()), next)
    }

    /// `reconstruct_chunk_table`: each entry from the chunk headers found by
    /// walking the file from the end of the tables; entries still unknown
    /// keep what the table said when any header was bad.
    fn reconstruct(&self, part_number: usize, table: &mut [u64]) -> Result<()> {
        let parts = &self.headers.parts;
        let last = &parts[parts.len() - 1];
        let mut offset_start = last.chunk_table_offset + 8 * last.chunk_count as u64;
        let part = &parts[part_number];
        let max_offset = self.file_size();
        if part_number > 0 {
            let previous = &parts[part_number - 1];
            let previous_table = self.read_table(part_number - 1)?;
            let mut chunk_start = offset_start;
            for &offset in &previous_table {
                if offset > chunk_start && offset < max_offset {
                    chunk_start = offset;
                }
            }
            offset_start = self.extract_leader(previous, part_number - 1, chunk_start)?.next_offset;
        }
        let count = part.chunk_count as usize;
        let mut rebuilt = vec![0u64; count];
        let mut first_failure: Option<anyhow::Error> = None;
        for (chunk, &table_offset) in table.iter().enumerate().take(count) {
            if table_offset >= offset_start && table_offset < max_offset {
                offset_start = table_offset;
            }
            let mut chunk_start = offset_start;
            let computed = if part.line_order() == LINE_ORDER_DECREASING_Y { count - (chunk + 1) } else { chunk };
            let mut found = computed as i32;
            let (result, next) = self.validate_leader(part, part_number, chunk_start, &mut found);
            if let Some(next) = next {
                offset_start = next;
            }
            if let Err(error) = result {
                chunk_start = 0;
                first_failure.get_or_insert(error);
            }
            if found >= 0 && (found as usize) < count && rebuilt[found as usize] == 0 {
                rebuilt[found as usize] = chunk_start;
            }
        }
        match first_failure {
            None => {
                table.copy_from_slice(&rebuilt);
                Ok(())
            }
            Some(error) => {
                for (entry, &value) in table.iter_mut().zip(rebuilt.iter()) {
                    if value != 0 {
                        *entry = value;
                    }
                }
                Err(error)
            }
        }
    }

    /// The part's table as read (and rebuilt when incomplete), without
    /// keeping it.
    fn read_table(&self, part_number: usize) -> Result<Vec<u64>> {
        let part = &self.headers.parts[part_number];
        let offset = part.chunk_table_offset;
        let bytes = 8 * part.chunk_count as u64;
        if part.chunk_count <= 0 {
            bail!("Invalid file with no chunks");
        }
        if part.chunk_count > 1024 * 1024 || bytes + offset > self.file_size() {
            bail!("chunk table size ({bytes}) too big for file size ({})", self.file_size());
        }
        let table_end = offset + bytes;
        let mut table: Vec<u64> = (0..part.chunk_count as u64).map(|index| read_u64(self.data, offset + 8 * index).unwrap_or(0)).collect();
        let complete = table.iter().all(|&entry| entry >= table_end && entry < self.file_size());
        if !complete {
            // Without strict checking a failed rebuild is let through.
            let _ = self.reconstruct(part_number, &mut table);
        }
        Ok(table)
    }

    /// `extract_chunk_table`: the table (kept), and where chunks may start.
    fn table(&mut self, part_number: usize) -> Result<(&[u64], u64)> {
        if self.tables[part_number].is_none() {
            let table = self.read_table(part_number).map_err(|error| error.to_string());
            self.tables[part_number] = Some(table);
        }
        let part = &self.headers.parts[part_number];
        let minimum = part.chunk_table_offset + 8 * part.chunk_count.max(0) as u64;
        match self.tables[part_number].as_ref().expect("the table was read") {
            Ok(table) => Ok((table.as_slice(), minimum)),
            Err(error) => Err(anyhow!("{error}")),
        }
    }

    /// `exr_read_scanline_chunk_info`: the chunk holding line `y`.
    pub fn scanline_chunk(&mut self, part_number: usize, y: i32) -> Result<ChunkInfo> {
        let multipart = self.headers.multipart;
        let file_size = self.file_size();
        let part = self.headers.parts[part_number].clone();
        if !part.storage.is_scanline() {
            bail!("Scanline / tile API usage mismatch");
        }
        if part.storage.is_deep() {
            bail!("deep data");
        }
        let window = part.data_window();
        if y < window.min_y || y > window.max_y {
            bail!("Invalid request for scanline {y} outside range of data window");
        }
        let lines = part.lines_per_chunk;
        let mut index = y - window.min_y;
        if lines > 1 {
            index /= lines;
        }
        let min_y = window.min_y + index * lines;
        if index < 0 || index >= part.chunk_count {
            bail!("Invalid request for scanline {y} in chunk {index} outside chunk count {}", part.chunk_count);
        }
        let width = window.max_x - window.min_x + 1;
        let mut height = lines;
        if min_y as i64 + lines as i64 > window.max_y as i64 {
            height = window.max_y - min_y + 1;
        }
        let (table, minimum) = self.table(part_number)?;
        let mut offset = table[index as usize];
        if offset == 0 {
            bail!("Incomplete chunk table");
        }
        if offset < minimum || offset > file_size {
            bail!("Corrupt chunk offset table: scanline {y}, chunk index {index} recorded at file offset {offset}");
        }
        let mut next = || -> Result<i32> {
            let value = read_i32(self.data, offset).ok_or_else(|| anyhow!("read past end of file"))?;
            offset += 4;
            Ok(value)
        };
        if multipart && next()? != part_number as i32 {
            bail!("Preparing read scanline {y} (chunk {index}), found corrupt leader: wrong part");
        }
        let leader_y = next()?;
        if leader_y != min_y {
            bail!("Preparing to read scanline {y} (chunk {index}), found corrupt leader: scanline says {leader_y}, expected {min_y}");
        }
        let packed = next()?;
        if packed < 0 || packed as u64 > part.unpacked_size_per_chunk {
            bail!("Preparing to read scanline {y} (chunk {index}), found corrupt leader: packed data size says {packed}");
        }
        let unpacked_size = chunk_unpack_size(&part, window.min_x, min_y, width, height, lines);
        if offset + packed as u64 > file_size {
            bail!("Preparing to read scanline {y} (chunk {index}), found corrupt leader: packed size {packed}");
        }
        if packed == 0 && unpacked_size > 0 {
            bail!("Invalid packed size of 0");
        }
        Ok(ChunkInfo { start_x: window.min_x, start_y: min_y, width, height, data_offset: offset, packed_size: packed as u64, unpacked_size })
    }

    /// `exr_read_tile_chunk_info` for a tile of level (0, 0).
    pub fn tile_chunk(&mut self, part_number: usize, tile_x: i32, tile_y: i32) -> Result<ChunkInfo> {
        let multipart = self.headers.multipart;
        let file_size = self.file_size();
        let part = self.headers.parts[part_number].clone();
        if !matches!(part.storage, Storage::Tiled | Storage::DeepTiled) {
            bail!("Scanline / tile API usage mismatch");
        }
        if part.storage.is_deep() {
            bail!("deep data");
        }
        let index = tile_chunk_index(&part, tile_x, tile_y, 0, 0)?;
        let tiles = part.tiles.expect("tiled parts have tiles");
        let clip = |size: u32, tile: i32, level_size: i32| -> i32 {
            let size = size as i32;
            let end = size as i64 * (tile as i64 + 1);
            if end > level_size as i64 {
                let over = end - level_size as i64;
                if over < size as i64 {
                    return size - over as i32;
                }
            }
            size
        };
        let width = clip(tiles.x_size, tile_x, part.tile_level_sizes.0[0]);
        let height = clip(tiles.y_size, tile_y, part.tile_level_sizes.1[0]);
        let texels = width as u64 * height as u64;
        let unpacked_size: u64 = part.channels().iter().map(|channel| texels * channel.bytes_per_element()).sum();
        let (table, minimum) = self.table(part_number)?;
        let mut offset = table[index as usize];
        if offset == 0 {
            bail!("Incomplete chunk table");
        }
        if offset < minimum || offset > file_size {
            bail!("Corrupt chunk offset table: tile ({tile_x}, {tile_y}), chunk index {index}");
        }
        let mut next = || -> Result<i32> {
            let value = read_i32(self.data, offset).ok_or_else(|| anyhow!("Unable to read information block for tile"))?;
            offset += 4;
            Ok(value)
        };
        if multipart && next()? != part_number as i32 {
            bail!("Corrupt tile: wrong part");
        }
        let leader = [next()?, next()?, next()?, next()?];
        if leader != [tile_x, tile_y, 0, 0] {
            bail!("Corrupt tile ({tile_x}, {tile_y}): bad tile coordinates or level");
        }
        let packed = next()?;
        if packed < 0 || packed as u64 > unpacked_size || (packed == 0 && unpacked_size != 0) {
            bail!("Corrupt tile ({tile_x}, {tile_y}): invalid packed size {packed} vs unpacked size {unpacked_size}");
        }
        if offset + packed as u64 > file_size {
            bail!("Corrupt tile ({tile_x}, {tile_y}): access past end of file");
        }
        if part.compression() == COMPRESSION_NONE && packed as u64 != unpacked_size {
            bail!("Mismatch between unpacked and packed size with uncompressed data");
        }
        Ok(ChunkInfo { start_x: tile_x, start_y: tile_y, width, height, data_offset: offset, packed_size: packed as u64, unpacked_size })
    }

    /// `exr_read_chunk`: the chunk's packed bytes.
    pub fn read(&self, chunk: &ChunkInfo) -> Result<&'a [u8]> {
        let start = chunk.data_offset as usize;
        let end = start + chunk.packed_size as usize;
        self.data.get(start..end).ok_or_else(|| anyhow!("Unable to read pixel data block"))
    }
}

/// `compute_chunk_unpack_size`.
fn chunk_unpack_size(part: &Part, x: i32, y: i32, width: i32, height: i32, lines: i32) -> u64 {
    if !part.chan_has_line_sampling && height == lines {
        return part.unpacked_size_per_chunk;
    }
    part.channels()
        .iter()
        .map(|channel| channel.bytes_per_element() * sampled_width(width, channel.x_sampling, x) as u64 * sampled_height(height, channel.y_sampling, y) as u64)
        .sum()
}

/// `validate_and_compute_tile_chunk_off`.
fn tile_chunk_index(part: &Part, x: i32, y: i32, level_x: i32, level_y: i32) -> Result<i32> {
    let (counts_x, counts_y) = &part.tile_level_counts;
    let Some(tiles) = part.tiles else {
        bail!("Tile descriptor data missing or corrupt");
    };
    if counts_x.is_empty() || counts_y.is_empty() {
        bail!("Tile descriptor data missing or corrupt");
    }
    if x < 0 || y < 0 || level_x < 0 || level_y < 0 {
        bail!("Invalid tile indices provided");
    }
    let mut offset = 0i64;
    if tiles.level_mode == TILE_RIPMAP_LEVELS {
        if level_x as usize >= counts_x.len() || level_y as usize >= counts_y.len() {
            bail!("Request for tile past available levels");
        }
        let (across, down) = (counts_x[level_x as usize], counts_y[level_y as usize]);
        if x >= across || y >= down {
            bail!("Request for tile outside the level");
        }
        for &count_y in counts_y.iter().take(level_y as usize) {
            for &count_x in counts_x {
                offset += count_x as i64 * count_y as i64;
            }
        }
        for &count_x in counts_x.iter().take(level_x as usize) {
            offset += count_x as i64 * down as i64;
        }
        offset += y as i64 * across as i64 + x as i64;
    } else {
        if level_x != level_y {
            bail!("single level and mipmap tiles must have same level x and y");
        }
        if level_x as usize >= counts_x.len() {
            bail!("Request for tile past available levels");
        }
        let (across, down) = (counts_x[level_x as usize], counts_y[level_x as usize]);
        if x >= across || y >= down {
            bail!("Request for tile outside the level");
        }
        for level in 0..level_x as usize {
            offset += counts_x[level] as i64 * counts_y[level] as i64;
        }
        offset += y as i64 * across as i64 + x as i64;
    }
    if offset >= part.chunk_count as i64 {
        bail!("Invalid tile chunk offset {offset} ({} avail)", part.chunk_count);
    }
    Ok(offset as i32)
}
