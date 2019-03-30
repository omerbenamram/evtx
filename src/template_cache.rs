use crate::binxml::deserializer::{Context};
use crate::binxml::tokens::read_template_definition;



use crate::model::deserialized::BinXMLTemplateDefinition;
use crate::Offset;
pub use byteorder::{LittleEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom};
use std::rc::Rc;

pub type CachedTemplate<'c> = (Rc<BinXMLTemplateDefinition<'c>>);

#[derive(Debug)]
pub struct TemplateCache<'c>(HashMap<Offset, CachedTemplate<'c>>);

impl<'c> TemplateCache<'c> {
    pub fn new() -> Self {
        TemplateCache(HashMap::new())
    }

    pub fn populate(&mut self, data: &'c [u8], offsets: &[Offset]) -> Result<(), failure::Error> {
        let mut cursor = Cursor::new(data);
        for offset in offsets.iter().filter(|&&offset| offset > 0) {
            cursor.seek(SeekFrom::Start(*offset as u64))?;
            let definition = read_template_definition(&mut cursor, Context::default())?;
            self.0.insert(*offset, Rc::new(definition));
        }

        Ok(())
    }

    pub fn get_template(&self, offset: Offset) -> Option<CachedTemplate<'c>> {
        self.0.get(&offset).and_then(|t| Some(Rc::clone(t)))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
