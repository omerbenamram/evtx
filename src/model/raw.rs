#[derive(Debug, PartialOrd, PartialEq, Eq)]
pub enum BinXMLRawToken {
    EndOfStream,
    // True if has attributes, otherwise false.
    OpenStartElement(OpenStartElementTokenMeta),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    Value,
    Attribute(AttributeTokenMeta),
    CDataSection,
    CharReference,
    EntityReference,
    ProcessingInstructionTarget,
    ProcessingInstructionData,
    TemplateInstance,
    NormalSubstitution,
    ConditionalSubstitution,
    StartOfStream,
}

impl BinXMLRawToken {
    pub fn from_u8(byte: u8) -> Option<BinXMLRawToken> {
        match byte {
            0x00 => Some(BinXMLRawToken::EndOfStream),
            // <Event>
            0x01 => Some(BinXMLRawToken::OpenStartElement(
                OpenStartElementTokenMeta {
                    has_attributes: false,
                },
            )),
            0x41 => Some(BinXMLRawToken::OpenStartElement(
                OpenStartElementTokenMeta {
                    has_attributes: true,
                },
            )),
            // Indicates end of start element
            0x02 => Some(BinXMLRawToken::CloseStartElement),
            0x03 => Some(BinXMLRawToken::CloseEmptyElement),
            // </Event>
            0x04 => Some(BinXMLRawToken::CloseElement),
            0x05 | 0x45 => Some(BinXMLRawToken::Value),
            0x06 => Some(BinXMLRawToken::Attribute(AttributeTokenMeta {
                more_attributes_expected: false,
            })),
            0x46 => Some(BinXMLRawToken::Attribute(AttributeTokenMeta {
                more_attributes_expected: true,
            })),
            0x07 | 0x47 => Some(BinXMLRawToken::CDataSection),
            0x08 | 0x48 => Some(BinXMLRawToken::CharReference),
            0x09 | 0x49 => Some(BinXMLRawToken::EntityReference),
            0x0a => Some(BinXMLRawToken::ProcessingInstructionTarget),
            0x0b => Some(BinXMLRawToken::ProcessingInstructionData),
            0x0c => Some(BinXMLRawToken::TemplateInstance),
            0x0d => Some(BinXMLRawToken::NormalSubstitution),
            0x0e => Some(BinXMLRawToken::ConditionalSubstitution),
            0x0f => Some(BinXMLRawToken::StartOfStream),
            _ => None,
        }
    }
}

#[derive(Debug, PartialOrd, PartialEq, Eq)]
pub struct OpenStartElementTokenMeta {
    pub has_attributes: bool,
}

#[derive(Debug, PartialOrd, PartialEq, Eq)]
pub struct AttributeTokenMeta {
    pub more_attributes_expected: bool,
}
