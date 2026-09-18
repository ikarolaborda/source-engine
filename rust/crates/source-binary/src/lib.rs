//! Checked little-endian and Source-style bit-buffer primitives.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    UnexpectedEof {
        offset: usize,
        requested: usize,
        remaining: usize,
    },
    InvalidSeek {
        offset: usize,
        length: usize,
    },
    UnterminatedString {
        offset: usize,
        limit: usize,
    },
    LimitExceeded {
        what: &'static str,
        value: usize,
        limit: usize,
    },
    InvalidBitCount(u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof {
                offset,
                requested,
                remaining,
            } => write!(
                f,
                "unexpected end at {offset}: requested {requested} bytes, {remaining} remain"
            ),
            Self::InvalidSeek { offset, length } => {
                write!(f, "offset {offset} is outside a {length}-byte buffer")
            }
            Self::UnterminatedString { offset, limit } => {
                write!(f, "unterminated string at {offset} (limit {limit})")
            }
            Self::LimitExceeded { what, value, limit } => {
                write!(f, "{what} value {value} exceeds limit {limit}")
            }
            Self::InvalidBitCount(count) => write!(f, "invalid bit count {count}; maximum is 64"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub fn with_position(bytes: &'a [u8], position: usize) -> Result<Self> {
        if position > bytes.len() {
            return Err(Error::InvalidSeek {
                offset: position,
                length: bytes.len(),
            });
        }
        Ok(Self { bytes, position })
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub fn seek(&mut self, position: usize) -> Result<()> {
        if position > self.bytes.len() {
            return Err(Error::InvalidSeek {
                offset: position,
                length: self.bytes.len(),
            });
        }
        self.position = position;
        Ok(())
    }

    pub fn skip(&mut self, count: usize) -> Result<()> {
        let position = self.position.checked_add(count).ok_or(Error::InvalidSeek {
            offset: usize::MAX,
            length: self.bytes.len(),
        })?;
        self.seek(position)
    }

    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if count > self.remaining() {
            return Err(Error::UnexpectedEof {
                offset: self.position,
                requested: count,
                remaining: self.remaining(),
            });
        }
        let start = self.position;
        self.position += count;
        Ok(&self.bytes[start..self.position])
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_u16_le(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn read_i16_le(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn read_u32_le(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn read_i32_le(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn read_u64_le(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub fn read_i64_le(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub fn read_f32_le(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32_le()?))
    }

    pub fn read_f64_le(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.read_u64_le()?))
    }

    pub fn read_cstr(&mut self, max_len: usize) -> Result<&'a [u8]> {
        let start = self.position;
        let search_len = self.remaining().min(max_len.saturating_add(1));
        let haystack = &self.bytes[start..start + search_len];
        let relative_end =
            haystack
                .iter()
                .position(|byte| *byte == 0)
                .ok_or(Error::UnterminatedString {
                    offset: start,
                    limit: max_len,
                })?;
        if relative_end > max_len {
            return Err(Error::LimitExceeded {
                what: "string length",
                value: relative_end,
                limit: max_len,
            });
        }
        self.position += relative_end + 1;
        Ok(&self.bytes[start..start + relative_end])
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    pub fn position(&self) -> usize {
        self.bytes.len()
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn write_u16_le(&mut self, value: u16) {
        self.write_bytes(&value.to_le_bytes());
    }

    pub fn write_i16_le(&mut self, value: i16) {
        self.write_bytes(&value.to_le_bytes());
    }

    pub fn write_u32_le(&mut self, value: u32) {
        self.write_bytes(&value.to_le_bytes());
    }

    pub fn write_i32_le(&mut self, value: i32) {
        self.write_bytes(&value.to_le_bytes());
    }

    pub fn write_u64_le(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    pub fn write_i64_le(&mut self, value: i64) {
        self.write_bytes(&value.to_le_bytes());
    }

    pub fn write_f32_le(&mut self, value: f32) {
        self.write_u32_le(value.to_bits());
    }

    pub fn write_f64_le(&mut self, value: f64) {
        self.write_u64_le(value.to_bits());
    }

    pub fn write_cstr(&mut self, value: &[u8]) {
        self.write_bytes(value);
        self.write_u8(0);
    }

    pub fn align(&mut self, alignment: usize) -> Result<()> {
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(Error::LimitExceeded {
                what: "alignment",
                value: alignment,
                limit: usize::MAX,
            });
        }
        let padding = self.bytes.len().wrapping_neg() & (alignment - 1);
        self.bytes.resize(self.bytes.len() + padding, 0);
        Ok(())
    }

    pub fn patch_u32_le(&mut self, offset: usize, value: u32) -> Result<()> {
        let length = self.bytes.len();
        let end = offset
            .checked_add(4)
            .ok_or(Error::InvalidSeek { offset, length })?;
        let target = self
            .bytes
            .get_mut(offset..end)
            .ok_or(Error::InvalidSeek { offset, length })?;
        target.copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct BitReader<'a> {
    bytes: &'a [u8],
    bit_position: usize,
    bit_len: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            bit_position: 0,
            bit_len: bytes.len().saturating_mul(8),
        }
    }

    pub fn with_bit_len(bytes: &'a [u8], bit_len: usize) -> Result<Self> {
        let available = bytes.len().saturating_mul(8);
        if bit_len > available {
            return Err(Error::UnexpectedEof {
                offset: 0,
                requested: bit_len,
                remaining: available,
            });
        }
        Ok(Self {
            bytes,
            bit_position: 0,
            bit_len,
        })
    }

    pub fn position(&self) -> usize {
        self.bit_position
    }

    pub fn len_bits(&self) -> usize {
        self.bit_len
    }

    pub fn remaining(&self) -> usize {
        self.bit_len.saturating_sub(self.bit_position)
    }

    /// Reads Source-style least-significant-bit-first fields.
    pub fn read_bits(&mut self, count: u32) -> Result<u64> {
        if count > 64 {
            return Err(Error::InvalidBitCount(count));
        }
        let count = count as usize;
        if count > self.remaining() {
            return Err(Error::UnexpectedEof {
                offset: self.bit_position,
                requested: count,
                remaining: self.remaining(),
            });
        }
        let mut value = 0u64;
        for output_bit in 0..count {
            let absolute_bit = self.bit_position + output_bit;
            let bit = (self.bytes[absolute_bit / 8] >> (absolute_bit % 8)) & 1;
            value |= u64::from(bit) << output_bit;
        }
        self.bit_position += count;
        Ok(value)
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_bits(1)? != 0)
    }

    pub fn peek_bits(&self, count: u32) -> Result<u64> {
        let mut copy = *self;
        copy.read_bits(count)
    }

    pub fn skip_bits(&mut self, count: usize) -> Result<()> {
        if count > self.remaining() {
            return Err(Error::UnexpectedEof {
                offset: self.bit_position,
                requested: count,
                remaining: self.remaining(),
            });
        }
        self.bit_position += count;
        Ok(())
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bit_len: usize,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len_bits(&self) -> usize {
        self.bit_len
    }

    pub fn is_empty(&self) -> bool {
        self.bit_len == 0
    }

    pub fn write_bits(&mut self, value: u64, count: u32) -> Result<()> {
        if count > 64 {
            return Err(Error::InvalidBitCount(count));
        }
        for input_bit in 0..count as usize {
            let absolute_bit = self.bit_len + input_bit;
            if absolute_bit / 8 == self.bytes.len() {
                self.bytes.push(0);
            }
            if ((value >> input_bit) & 1) != 0 {
                self.bytes[absolute_bit / 8] |= 1 << (absolute_bit % 8);
            }
        }
        self.bit_len += count as usize;
        Ok(())
    }

    pub fn write_bool(&mut self, value: bool) -> Result<()> {
        self.write_bits(u64::from(value), 1)
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endian_round_trip_and_bounds() {
        let mut writer = Writer::new();
        writer.write_u16_le(0x1234);
        writer.write_i32_le(-42);
        writer.write_f32_le(1.25);
        writer.write_cstr(b"source");

        let bytes = writer.into_inner();
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.read_u16_le().unwrap(), 0x1234);
        assert_eq!(reader.read_i32_le().unwrap(), -42);
        assert_eq!(reader.read_f32_le().unwrap(), 1.25);
        assert_eq!(reader.read_cstr(6).unwrap(), b"source");
        assert!(reader.is_empty());
        assert!(matches!(reader.read_u8(), Err(Error::UnexpectedEof { .. })));
    }

    #[test]
    fn strings_are_bounded() {
        let mut reader = Reader::new(b"abcdef\0");
        assert!(matches!(
            reader.read_cstr(3),
            Err(Error::UnterminatedString { .. }) | Err(Error::LimitExceeded { .. })
        ));
    }

    #[test]
    fn source_bit_order_round_trips() {
        let mut writer = BitWriter::new();
        writer.write_bits(0b101, 3).unwrap();
        writer.write_bits(0x12f, 9).unwrap();
        writer.write_bool(true).unwrap();
        assert_eq!(writer.len_bits(), 13);

        let mut reader = BitReader::new(writer.as_slice());
        assert_eq!(reader.read_bits(3).unwrap(), 0b101);
        assert_eq!(reader.read_bits(9).unwrap(), 0x12f);
        assert!(reader.read_bool().unwrap());
        assert_eq!(reader.remaining(), 3);

        let mut reader = BitReader::with_bit_len(writer.as_slice(), 13).unwrap();
        assert_eq!(reader.peek_bits(3).unwrap(), 0b101);
        assert_eq!(reader.position(), 0);
        reader.skip_bits(12).unwrap();
        assert!(reader.read_bool().unwrap());
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn patch_and_align_are_checked() {
        let mut writer = Writer::new();
        writer.write_u32_le(0);
        writer.write_u8(1);
        writer.align(4).unwrap();
        writer.patch_u32_le(0, 0xfeed_beef).unwrap();
        assert_eq!(writer.as_slice(), &[0xef, 0xbe, 0xed, 0xfe, 1, 0, 0, 0]);
        assert!(writer.patch_u32_le(6, 0).is_err());
    }
}
