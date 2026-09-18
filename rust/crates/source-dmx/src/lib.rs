//! Bounded reader for Source Data Model Exchange (DMX) binary encodings.
//!
//! Particle collection (`.pcf`) files in the frozen HL2 corpus use
//! `binary 2 / pcf 1`. The reader also handles binary encodings 0 and 1,
//! whose element and attribute names are stored inline instead of in a symbol
//! table.

use source_binary::Reader;
use std::fmt;

const HEADER_PREFIX: &str = "<!-- dmx";
const HEADER_END: &str = "-->";
const MAX_HEADER_SIZE: usize = 168;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_string_size: usize,
    pub max_elements: usize,
    pub max_attributes: usize,
    pub max_array_values: usize,
    pub max_blob_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 512 * 1024 * 1024,
            max_string_size: 1024 * 1024,
            max_elements: 1_000_000,
            max_attributes: 4_000_000,
            max_array_values: 16_000_000,
            max_blob_size: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header<'a> {
    pub encoding: &'a str,
    pub encoding_version: u32,
    pub format: &'a str,
    pub format_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Name<'a> {
    Symbol(u16),
    Inline(&'a [u8]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementRef<'a> {
    Null,
    Local(usize),
    External(&'a [u8]),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Element(ElementRef<'a>),
    Int(i32),
    Float(f32),
    Bool(bool),
    String(&'a [u8]),
    Void(&'a [u8]),
    ObjectId([u8; 16]),
    Color([u8; 4]),
    Vector2([f32; 2]),
    Vector3([f32; 3]),
    Vector4([f32; 4]),
    QAngle([f32; 3]),
    Quaternion([f32; 4]),
    Matrix([f32; 16]),
    Array(Vec<Value<'a>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute<'a> {
    pub name: Name<'a>,
    pub kind: AttributeType,
    pub value: Value<'a>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element<'a> {
    pub element_type: Name<'a>,
    pub name: &'a [u8],
    pub id: [u8; 16],
    pub attributes: Vec<Attribute<'a>>,
}

#[derive(Debug, Clone)]
pub struct Document<'a> {
    bytes: &'a [u8],
    header: Header<'a>,
    symbols: Vec<&'a [u8]>,
    elements: Vec<Element<'a>>,
}

impl<'a> Document<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        if bytes.len() > limits.max_file_size {
            return Err(Error::LimitExceeded {
                what: "file size",
                value: bytes.len(),
                limit: limits.max_file_size,
            });
        }
        let (header, body_offset) = parse_header(bytes)?;
        if header.encoding != "binary" {
            return Err(Error::UnsupportedEncoding(header.encoding.to_owned()));
        }
        if header.encoding_version > 2 {
            return Err(Error::UnsupportedEncodingVersion(header.encoding_version));
        }

        let mut reader = Reader::with_position(bytes, body_offset)?;
        let symbols = if header.encoding_version >= 2 {
            let count = usize::from(reader.read_u16_le()?);
            let mut symbols = Vec::with_capacity(count);
            for _ in 0..count {
                symbols.push(reader.read_cstr(limits.max_string_size)?);
            }
            symbols
        } else {
            Vec::new()
        };

        let element_count = count("element count", reader.read_i32_le()?, limits.max_elements)?;
        let mut elements = Vec::with_capacity(element_count);
        for index in 0..element_count {
            let offset = reader.position();
            let parsed = (|| {
                let element_type = read_name(&mut reader, &symbols, &header, limits)?;
                let name = reader.read_cstr(limits.max_string_size)?;
                let id = reader.take(16)?.try_into().expect("16-byte object id");
                Ok((element_type, name, id))
            })()
            .map_err(|source| Error::ElementDictionary {
                index,
                offset,
                source: Box::new(source),
            })?;
            elements.push(Element {
                element_type: parsed.0,
                name: parsed.1,
                id: parsed.2,
                attributes: Vec::new(),
            });
        }

        let mut total_attributes = 0usize;
        let mut total_array_values = 0usize;
        for (element_index, element) in elements.iter_mut().enumerate() {
            let attribute_count = count(
                "element attribute count",
                reader.read_i32_le()?,
                limits.max_attributes,
            )?;
            total_attributes = checked_total(
                "total attribute count",
                total_attributes,
                attribute_count,
                limits.max_attributes,
            )?;
            element.attributes.reserve(attribute_count);
            for attribute_index in 0..attribute_count {
                let offset = reader.position();
                let attribute = (|| {
                    let name = read_name(&mut reader, &symbols, &header, limits)?;
                    let raw_kind = reader.read_u8()?;
                    let kind = AttributeType::try_from(raw_kind)
                        .map_err(|()| Error::InvalidAttributeType(raw_kind))?;
                    let value = read_value(
                        &mut reader,
                        kind,
                        element_count,
                        limits,
                        &mut total_array_values,
                    )?;
                    Ok(Attribute { name, kind, value })
                })()
                .map_err(|source| Error::Attribute {
                    element: element_index,
                    attribute: attribute_index,
                    offset,
                    source: Box::new(source),
                })?;
                element.attributes.push(attribute);
            }
        }
        if !reader.is_empty() {
            return Err(Error::TrailingData(reader.remaining()));
        }

        Ok(Self {
            bytes,
            header,
            symbols,
            elements,
        })
    }

    pub fn header(&self) -> &Header<'a> {
        &self.header
    }

    pub fn symbols(&self) -> &[&'a [u8]] {
        &self.symbols
    }

    pub fn elements(&self) -> &[Element<'a>] {
        &self.elements
    }

    pub fn original_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn resolve_name(&self, name: Name<'a>) -> Option<&'a [u8]> {
        match name {
            Name::Symbol(index) => self.symbols.get(usize::from(index)).copied(),
            Name::Inline(value) => Some(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AttributeType {
    Element = 1,
    Int = 2,
    Float = 3,
    Bool = 4,
    String = 5,
    Void = 6,
    ObjectId = 7,
    Color = 8,
    Vector2 = 9,
    Vector3 = 10,
    Vector4 = 11,
    QAngle = 12,
    Quaternion = 13,
    Matrix = 14,
    ElementArray = 15,
    IntArray = 16,
    FloatArray = 17,
    BoolArray = 18,
    StringArray = 19,
    VoidArray = 20,
    ObjectIdArray = 21,
    ColorArray = 22,
    Vector2Array = 23,
    Vector3Array = 24,
    Vector4Array = 25,
    QAngleArray = 26,
    QuaternionArray = 27,
    MatrixArray = 28,
}

impl AttributeType {
    fn scalar_for_array(self) -> Option<Self> {
        let raw = self as u8;
        (raw >= Self::ElementArray as u8)
            .then(|| Self::try_from(raw - 14).expect("array types map to scalar types"))
    }
}

impl TryFrom<u8> for AttributeType {
    type Error = ();

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            1 => Self::Element,
            2 => Self::Int,
            3 => Self::Float,
            4 => Self::Bool,
            5 => Self::String,
            6 => Self::Void,
            7 => Self::ObjectId,
            8 => Self::Color,
            9 => Self::Vector2,
            10 => Self::Vector3,
            11 => Self::Vector4,
            12 => Self::QAngle,
            13 => Self::Quaternion,
            14 => Self::Matrix,
            15 => Self::ElementArray,
            16 => Self::IntArray,
            17 => Self::FloatArray,
            18 => Self::BoolArray,
            19 => Self::StringArray,
            20 => Self::VoidArray,
            21 => Self::ObjectIdArray,
            22 => Self::ColorArray,
            23 => Self::Vector2Array,
            24 => Self::Vector3Array,
            25 => Self::Vector4Array,
            26 => Self::QAngleArray,
            27 => Self::QuaternionArray,
            28 => Self::MatrixArray,
            _ => return Err(()),
        })
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    InvalidHeader,
    InvalidHeaderUtf8,
    InvalidHeaderVersion,
    UnsupportedEncoding(String),
    UnsupportedEncodingVersion(u32),
    InvalidCount {
        what: &'static str,
        value: i32,
    },
    LimitExceeded {
        what: &'static str,
        value: usize,
        limit: usize,
    },
    InvalidSymbolIndex {
        index: u16,
        count: usize,
    },
    InvalidAttributeType(u8),
    InvalidElementIndex {
        index: i32,
        count: usize,
    },
    ElementDictionary {
        index: usize,
        offset: usize,
        source: Box<Error>,
    },
    Attribute {
        element: usize,
        attribute: usize,
        offset: usize,
        source: Box<Error>,
    },
    TrailingData(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::InvalidHeader => write!(f, "invalid DMX header"),
            Self::InvalidHeaderUtf8 => write!(f, "DMX header is not valid UTF-8"),
            Self::InvalidHeaderVersion => write!(f, "invalid DMX header version"),
            Self::UnsupportedEncoding(value) => write!(f, "unsupported DMX encoding {value}"),
            Self::UnsupportedEncodingVersion(value) => {
                write!(f, "unsupported DMX binary encoding version {value}")
            }
            Self::InvalidCount { what, value } => write!(f, "invalid DMX {what} {value}"),
            Self::LimitExceeded { what, value, limit } => {
                write!(f, "DMX {what} {value} exceeds limit {limit}")
            }
            Self::InvalidSymbolIndex { index, count } => {
                write!(
                    f,
                    "DMX symbol index {index} is outside a {count}-entry table"
                )
            }
            Self::InvalidAttributeType(value) => write!(f, "invalid DMX attribute type {value}"),
            Self::InvalidElementIndex { index, count } => {
                write!(f, "DMX element index {index} is outside {count} elements")
            }
            Self::ElementDictionary {
                index,
                offset,
                source,
            } => write!(
                f,
                "DMX element dictionary entry {index} at byte {offset}: {source}"
            ),
            Self::Attribute {
                element,
                attribute,
                offset,
                source,
            } => write!(
                f,
                "DMX element {element} attribute {attribute} at byte {offset}: {source}"
            ),
            Self::TrailingData(size) => write!(f, "DMX file has {size} trailing bytes"),
        }
    }
}

impl std::error::Error for Error {}

impl From<source_binary::Error> for Error {
    fn from(value: source_binary::Error) -> Self {
        Self::Binary(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn parse_header(bytes: &[u8]) -> Result<(Header<'_>, usize)> {
    let search = &bytes[..bytes.len().min(MAX_HEADER_SIZE)];
    let end = search
        .windows(HEADER_END.len())
        .position(|window| window == HEADER_END.as_bytes())
        .ok_or(Error::InvalidHeader)?;
    let text = std::str::from_utf8(&search[..end]).map_err(|_| Error::InvalidHeaderUtf8)?;
    if !text.starts_with(HEADER_PREFIX) {
        return Err(Error::InvalidHeader);
    }
    let body = text[HEADER_PREFIX.len()..end].trim();
    let fields: Vec<_> = body.split_ascii_whitespace().collect();
    let [encoding_keyword, encoding, encoding_version, format_keyword, format, format_version] =
        fields.as_slice()
    else {
        return Err(Error::InvalidHeader);
    };
    if *encoding_keyword != "encoding" || *format_keyword != "format" {
        return Err(Error::InvalidHeader);
    }
    let encoding_version = encoding_version
        .parse()
        .map_err(|_| Error::InvalidHeaderVersion)?;
    let format_version = format_version
        .parse()
        .map_err(|_| Error::InvalidHeaderVersion)?;
    let mut body_offset = end + HEADER_END.len();
    match bytes.get(body_offset..body_offset.saturating_add(2)) {
        Some(b"\r\n") => body_offset += 2,
        _ if bytes.get(body_offset) == Some(&b'\n') => body_offset += 1,
        _ => {}
    }
    // CUtlBuffer::Printf delegates to PutString; binary buffers therefore
    // append a NUL after the textual header line.
    if bytes.get(body_offset) == Some(&0) {
        body_offset += 1;
    }
    Ok((
        Header {
            encoding,
            encoding_version,
            format,
            format_version,
        },
        body_offset,
    ))
}

fn read_name<'a>(
    reader: &mut Reader<'a>,
    symbols: &[&'a [u8]],
    header: &Header<'_>,
    limits: Limits,
) -> Result<Name<'a>> {
    if header.encoding_version >= 2 {
        let index = reader.read_u16_le()?;
        if usize::from(index) >= symbols.len() {
            return Err(Error::InvalidSymbolIndex {
                index,
                count: symbols.len(),
            });
        }
        Ok(Name::Symbol(index))
    } else {
        Ok(Name::Inline(reader.read_cstr(limits.max_string_size)?))
    }
}

fn read_value<'a>(
    reader: &mut Reader<'a>,
    kind: AttributeType,
    element_count: usize,
    limits: Limits,
    total_array_values: &mut usize,
) -> Result<Value<'a>> {
    if let Some(scalar) = kind.scalar_for_array() {
        let value_count = count(
            "array value count",
            reader.read_i32_le()?,
            limits.max_array_values,
        )?;
        *total_array_values = checked_total(
            "total array value count",
            *total_array_values,
            value_count,
            limits.max_array_values,
        )?;
        let mut values = Vec::with_capacity(value_count);
        for _ in 0..value_count {
            values.push(read_scalar(reader, scalar, element_count, limits)?);
        }
        Ok(Value::Array(values))
    } else {
        read_scalar(reader, kind, element_count, limits)
    }
}

fn read_scalar<'a>(
    reader: &mut Reader<'a>,
    kind: AttributeType,
    element_count: usize,
    limits: Limits,
) -> Result<Value<'a>> {
    Ok(match kind {
        AttributeType::Element => Value::Element(read_element_ref(reader, element_count, limits)?),
        AttributeType::Int => Value::Int(reader.read_i32_le()?),
        AttributeType::Float => Value::Float(reader.read_f32_le()?),
        AttributeType::Bool => Value::Bool(reader.read_u8()? != 0),
        AttributeType::String => Value::String(reader.read_cstr(limits.max_string_size)?),
        AttributeType::Void => {
            let size = count("blob size", reader.read_i32_le()?, limits.max_blob_size)?;
            Value::Void(reader.take(size)?)
        }
        AttributeType::ObjectId => {
            Value::ObjectId(reader.take(16)?.try_into().expect("16-byte object id"))
        }
        AttributeType::Color => Value::Color(reader.take(4)?.try_into().expect("four-byte color")),
        AttributeType::Vector2 => Value::Vector2(read_floats(reader)?),
        AttributeType::Vector3 => Value::Vector3(read_floats(reader)?),
        AttributeType::Vector4 => Value::Vector4(read_floats(reader)?),
        AttributeType::QAngle => Value::QAngle(read_floats(reader)?),
        AttributeType::Quaternion => Value::Quaternion(read_floats(reader)?),
        AttributeType::Matrix => Value::Matrix(read_floats(reader)?),
        _ => unreachable!("array types are handled before scalar parsing"),
    })
}

fn read_floats<const N: usize>(reader: &mut Reader<'_>) -> Result<[f32; N]> {
    let mut values = [0.0; N];
    for value in &mut values {
        *value = reader.read_f32_le()?;
    }
    Ok(values)
}

fn read_element_ref<'a>(
    reader: &mut Reader<'a>,
    element_count: usize,
    limits: Limits,
) -> Result<ElementRef<'a>> {
    let index = reader.read_i32_le()?;
    match index {
        -1 => Ok(ElementRef::Null),
        -2 => Ok(ElementRef::External(
            reader.read_cstr(limits.max_string_size.min(40))?,
        )),
        value if value >= 0 && (value as usize) < element_count => {
            Ok(ElementRef::Local(value as usize))
        }
        _ => Err(Error::InvalidElementIndex {
            index,
            count: element_count,
        }),
    }
}

fn count(what: &'static str, value: i32, limit: usize) -> Result<usize> {
    let value = usize::try_from(value).map_err(|_| Error::InvalidCount { what, value })?;
    if value > limit {
        return Err(Error::LimitExceeded { what, value, limit });
    }
    Ok(value)
}

fn checked_total(
    what: &'static str,
    current: usize,
    addition: usize,
    limit: usize,
) -> Result<usize> {
    let value = current.saturating_add(addition);
    if value > limit {
        return Err(Error::LimitExceeded { what, value, limit });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_binary::Writer;

    fn sample() -> Vec<u8> {
        let mut writer = Writer::new();
        writer.write_bytes(b"<!-- dmx encoding binary 2 format pcf 1 -->\n\0");
        writer.write_u16_le(3);
        writer.write_cstr(b"DmElement");
        writer.write_cstr(b"answer");
        writer.write_cstr(b"links");
        writer.write_i32_le(1);
        writer.write_u16_le(0);
        writer.write_cstr(b"root");
        writer.write_bytes(&[7; 16]);
        writer.write_i32_le(2);
        writer.write_u16_le(1);
        writer.write_u8(AttributeType::Int as u8);
        writer.write_i32_le(42);
        writer.write_u16_le(2);
        writer.write_u8(AttributeType::ElementArray as u8);
        writer.write_i32_le(2);
        writer.write_i32_le(0);
        writer.write_i32_le(-1);
        writer.into_inner()
    }

    #[test]
    fn parses_binary_two_document() {
        let bytes = sample();
        let document = Document::parse(&bytes).unwrap();
        assert_eq!(document.header().format, "pcf");
        assert_eq!(
            document.resolve_name(document.elements()[0].element_type),
            Some(&b"DmElement"[..])
        );
        assert_eq!(document.elements()[0].name, b"root");
        assert_eq!(document.elements()[0].attributes[0].value, Value::Int(42));
        assert!(matches!(
            &document.elements()[0].attributes[1].value,
            Value::Array(values) if values == &vec![Value::Element(ElementRef::Local(0)), Value::Element(ElementRef::Null)]
        ));
        assert_eq!(document.original_bytes(), bytes);
    }

    #[test]
    fn rejects_bad_symbol_and_trailing_data() {
        let mut bytes = sample();
        let type_symbol_offset = b"<!-- dmx encoding binary 2 format pcf 1 -->\n\0".len()
            + 2
            + b"DmElement\0answer\0links\0".len()
            + 4;
        bytes[type_symbol_offset..type_symbol_offset + 2].copy_from_slice(&9u16.to_le_bytes());
        assert!(matches!(
            Document::parse(&bytes),
            Err(Error::ElementDictionary { source, .. })
                if matches!(*source, Error::InvalidSymbolIndex { index: 9, .. })
        ));

        let mut bytes = sample();
        bytes.push(0);
        assert!(matches!(
            Document::parse(&bytes),
            Err(Error::TrailingData(1))
        ));
    }

    #[test]
    fn enforces_aggregate_array_limit() {
        let bytes = sample();
        let limits = Limits {
            max_array_values: 1,
            ..Limits::default()
        };
        assert!(matches!(
            Document::parse_with_limits(&bytes, limits),
            Err(Error::Attribute { source, .. })
                if matches!(*source, Error::LimitExceeded {
                    what: "array value count",
                    ..
                })
        ));
    }
}
