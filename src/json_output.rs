use crate::err::{SerializationError, SerializationResult};

use crate::ParserSettings;
use crate::binxml::value_variant::BinXmlValue;
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::xml_output::BinXmlOutput;

use core::borrow::BorrowMut;
use log::trace;
use serde_json::{Map, Value, json};
use std::borrow::Cow;

use crate::binxml::name::BinXmlName;
use crate::err::SerializationError::JsonStructureError;
use quick_xml::events::BytesText;

use hashbrown::HashMap as FastMap;

pub struct JsonOutput {
    map: Value,
    stack: Vec<String>,
    separate_json_attributes: bool,
    // Per-parent map of duplicate counters for child keys to avoid repeated linear scans
    dup_counters_stack: Vec<FastMap<String, usize, ahash::RandomState>>,
}

impl JsonOutput {
    pub fn new(settings: &ParserSettings) -> Self {
        JsonOutput {
            map: Value::Object(Map::new()),
            stack: vec![],
            separate_json_attributes: settings.should_separate_json_attributes(),
            dup_counters_stack: Vec::new(),
        }
    }

    /// Return the per-parent duplicate counter map for the current parent depth
    fn parent_dup_map(&mut self) -> &mut FastMap<String, usize, ahash::RandomState> {
        let parent_depth = self.stack.len().saturating_sub(1);
        if self.dup_counters_stack.len() <= parent_depth {
            // Ensure root exists and maintain alignment with stack depth
            while self.dup_counters_stack.len() <= parent_depth {
                self.dup_counters_stack
                    .push(FastMap::with_hasher(ahash::RandomState::new()));
            }
        }
        &mut self.dup_counters_stack[parent_depth]
    }

    /// Compute or fetch the next duplicate index for `base` under current parent, scanning at most once.
    #[allow(dead_code)]
    fn next_duplicate_index_for(&mut self, container: &Map<String, Value>, base: &str) -> usize {
        let dup_map = self.parent_dup_map();
        if let Some(next) = dup_map.get(base) {
            return *next;
        }
        // First duplication for this key in this parent: scan once to find max suffix
        let mut max_idx = 1usize;
        let prefix = base;
        let mut tmp = String::with_capacity(prefix.len() + 1);
        tmp.push_str(prefix);
        tmp.push('_');
        let pref = tmp; // now pref = "base_"
        for k in container.keys() {
            if let Some(rest) = k.strip_prefix(&pref) {
                if let Some((num_part, attr_rest)) = rest.split_once("_") {
                    // Might be base_N_attributes -> consider N
                    if attr_rest == "attributes" {
                        if let Ok(n) = num_part.parse::<usize>() {
                            if n >= max_idx {
                                max_idx = n + 1;
                            }
                        }
                        continue;
                    }
                }
                // Otherwise keys like base_N
                if let Ok(n) = rest.parse::<usize>() {
                    if n >= max_idx {
                        max_idx = n + 1;
                    }
                }
            }
        }
        dup_map.insert(base.to_owned(), max_idx);
        max_idx
    }

    fn next_duplicate_index_from_keys(&mut self, keys: &[String], base: &str) -> usize {
        let dup_map = self.parent_dup_map();
        if let Some(next) = dup_map.get(base) {
            return *next;
        }
        let mut max_idx = 1usize;
        let pref = format!("{}_", base);
        for k in keys {
            if let Some(rest) = k.strip_prefix(&pref) {
                if let Some((num_part, attr_rest)) = rest.split_once("_") {
                    if attr_rest == "attributes" {
                        if let Ok(n) = num_part.parse::<usize>() {
                            if n >= max_idx {
                                max_idx = n + 1;
                            }
                        }
                        continue;
                    }
                }
                if let Ok(n) = rest.parse::<usize>() {
                    if n >= max_idx {
                        max_idx = n + 1;
                    }
                }
            }
        }
        dup_map.insert(base.to_owned(), max_idx);
        max_idx
    }

    /// Advance the counter for `base` under current parent after using it.
    fn advance_duplicate_index(&mut self, base: &str) {
        let dup_map = self.parent_dup_map();
        let entry = dup_map.entry(base.to_owned()).or_insert(1);
        *entry += 1;
    }

    /// Looks up the current path, will fill with empty objects if needed.
    fn get_or_create_current_path(&mut self) -> &mut Value {
        let mut v_temp = self.map.borrow_mut();

        for key in self.stack.iter() {
            // Current path does not exist yet, we need to create it.
            if v_temp.get(key).is_none() {
                // Can happen if we have
                // <Event>
                //    <System>
                //       <...>
                // since system has no attributes it has null and not an empty map.
                if v_temp.is_null() {
                    let mut map = Map::with_capacity(1);
                    map.insert(key.clone(), Value::Object(Map::new()));
                    *v_temp = Value::Object(map);
                } else if !v_temp.is_object() {
                    // This branch could only happen while `separate-json-attributes` was on,
                    // and a very non-standard xml structure is going on (character nodes between XML nodes)
                    //
                    // Example:
                    // ```
                    //  <URLCacheFlushInfo></URLCacheFlushInfo>&amp;quot&amp;<URLCacheResponseInfo></URLCacheResponseInfo>
                    // ```
                    // We shift the characters in to be consistent with regular json parser.
                    // The resulting JSON looks like:
                    // ```
                    // ...
                    //  "URLCacheResponseInfo": "\"",
                    //  "URLCacheResponseInfo_attributes": {
                    //      ...
                    //   }
                    // ...
                    // ```
                    let mut map = Map::with_capacity(1);
                    map.insert(key.clone(), v_temp.clone());
                    *v_temp = Value::Object(map);
                } else {
                    let current_object = v_temp
                        .as_object_mut()
                        .expect("!v_temp.is_object was matched above.");
                    current_object
                        .entry(key.clone())
                        .or_insert_with(|| Value::Object(Map::new()));
                }
            }

            v_temp = v_temp.get_mut(key).expect("Loop above inserted this node.")
        }

        v_temp
    }

    fn get_current_parent(&mut self) -> &mut Value {
        // Make sure we are operating on created nodes.
        self.get_or_create_current_path();

        let mut v_temp = self.map.borrow_mut();

        for key in self.stack.iter().take(self.stack.len() - 1) {
            v_temp = v_temp
                .get_mut(key)
                .expect("Calling `get_or_create_current_path` ensures that the node was created")
        }

        v_temp
    }

    /// Like a regular node, but uses it's "Name" attribute.
    fn insert_data_node(&mut self, element: &XmlElement) -> SerializationResult<()> {
        trace!("inserting data node {:?}", &element);
        match element
            .attributes
            .iter()
            .find(|a| a.name.as_ref().as_str() == "Name")
        {
            Some(name) => {
                let data_key: Cow<'_, str> = name.value.as_ref().as_cow_str();

                self.insert_node_without_attributes(element, &data_key)
            }
            // Ignore this node
            None => {
                self.stack.push("Data".to_owned());
                Ok(())
            }
        }
    }

    fn insert_node_without_attributes(
        &mut self,
        _e: &XmlElement,
        name: &str,
    ) -> SerializationResult<()> {
        trace!("insert_node_without_attributes");
        self.stack.push(name.to_owned());
        if self.dup_counters_stack.len() < self.stack.len() {
            self.dup_counters_stack
                .push(FastMap::with_hasher(ahash::RandomState::new()));
        }

        // Stage A: insert placeholder and capture old value + current keys
        let (old_value, keys): (Option<Value>, Vec<String>) = {
            let parent = self.get_current_parent();
            let parent_obj = parent.as_object_mut().ok_or_else(|| {
                SerializationError::JsonStructureError { message:
                "This is a bug - expected parent container to exist, and to be an object type.\
                          Check that the referencing parent is not `Value::null`".to_string(), }
            })?;
            let old_value = parent_obj.insert(name.to_string(), Value::Null);
            let keys = parent_obj.keys().cloned().collect();
            (old_value, keys)
        };

        if let Some(old_value) = old_value {
            if old_value.is_null() {
                return Ok(());
            }
            if let Some(map) = old_value.as_object() {
                if map.is_empty() {
                    return Ok(());
                }
            }

            let next_idx = self.next_duplicate_index_from_keys(&keys, name);

            // Stage B: move old value to suffixed key
            {
                let parent = self.get_current_parent();
                let parent_obj = parent.as_object_mut().ok_or_else(|| {
                    SerializationError::JsonStructureError {
                        message: "Expected parent to be an object".to_string(),
                    }
                })?;
                let dup_key = format!("{}_{}", name, next_idx);
                parent_obj.insert(dup_key, old_value);
            }

            // After map borrows are dropped, advance counter
            self.advance_duplicate_index(name);
        }

        Ok(())
    }

    fn insert_node_with_attributes(
        &mut self,
        element: &XmlElement,
        name: &str,
    ) -> SerializationResult<()> {
        trace!("insert_node_with_attributes");
        self.stack.push(name.to_owned());
        if self.dup_counters_stack.len() < self.stack.len() {
            self.dup_counters_stack
                .push(FastMap::with_hasher(ahash::RandomState::new()));
        }

        let mut attributes = Map::with_capacity(element.attributes.len());
        for attribute in element.attributes.iter() {
            let value = attribute.value.clone().into_owned();
            let value: Value = value.into();
            if !value.is_null() {
                let name: &str = attribute.name.as_str();
                attributes.insert(name.to_owned(), value);
            }
        }

        if !attributes.is_empty() {
            if self.separate_json_attributes {
                // Stage A: insert placeholders for attributes and name, capture old values and keys
                let (old_attr, old_val, keys): (Option<Value>, Option<Value>, Vec<String>) = {
                    let attr_key = format!("{}_attributes", name);
                    let parent = self.get_current_parent();
                    let parent_obj = parent.as_object_mut().ok_or_else(|| SerializationError::JsonStructureError { message:
                        "This is a bug - expected current value to exist, and to be an object type.\n                        Check that the value is not `Value::null`".to_string(), })?;
                    let old_attr = parent_obj.insert(attr_key, Value::Null);
                    let old_val = parent_obj.insert(name.to_string(), Value::Null);
                    let keys = parent_obj.keys().cloned().collect();
                    (old_attr, old_val, keys)
                };

                let next_idx = self.next_duplicate_index_from_keys(&keys, name);

                // Stage B: move old values to suffixed keys if non-empty objects
                {
                    let parent = self.get_current_parent();
                    let parent_obj = parent.as_object_mut().ok_or_else(|| {
                        SerializationError::JsonStructureError {
                            message: "Expected parent to be an object".to_string(),
                        }
                    })?;

                    if let Some(v) = old_val {
                        if v.as_object().map(|m| !m.is_empty()).unwrap_or(false) {
                            let value_key = format!("{}_{}", name, next_idx);
                            parent_obj.insert(value_key, v);
                        }
                    }
                    if let Some(a) = old_attr {
                        if a.as_object().map(|m| !m.is_empty()).unwrap_or(false) {
                            let attr_dup_key = format!("{}_{}_attributes", name, next_idx);
                            parent_obj.insert(attr_dup_key, a);
                        }
                    }
                }

                // Advance counter now that moves are complete
                self.advance_duplicate_index(name);

                // Stage C: set current attributes object and clean empty value placeholder
                {
                    let attr_key = format!("{}_attributes", name);
                    let parent = self.get_current_parent();
                    let parent_obj = parent.as_object_mut().ok_or_else(|| {
                        SerializationError::JsonStructureError {
                            message: "Expected parent to be an object".to_string(),
                        }
                    })?;
                    parent_obj.insert(attr_key, Value::Object(attributes));
                    if parent_obj
                        .get(name)
                        .map(|v| {
                            v.is_null() || v.as_object().map(|m| m.is_empty()).unwrap_or(false)
                        })
                        .unwrap_or(false)
                    {
                        parent_obj.remove(name);
                    }
                }
            } else {
                // Stage A: insert placeholder for name, capture old value and keys
                let (old_val, keys): (Option<Value>, Vec<String>) = {
                    let parent = self.get_current_parent();
                    let parent_obj = parent.as_object_mut().ok_or_else(|| SerializationError::JsonStructureError { message:
                        "This is a bug - expected parent container to exist, and to be an object type.\
                                Check that the referencing parent is not `Value::null`".to_string(),})?;
                    let old_val = parent_obj.insert(name.to_string(), Value::Null);
                    let keys = parent_obj.keys().cloned().collect();
                    (old_val, keys)
                };

                let next_idx = self.next_duplicate_index_from_keys(&keys, name);

                // Stage B: move old value if non-empty object and set new attributes object
                {
                    let parent = self.get_current_parent();
                    let parent_obj = parent.as_object_mut().ok_or_else(|| {
                        SerializationError::JsonStructureError {
                            message: "Expected parent to be an object".to_string(),
                        }
                    })?;

                    let mut needs_advance = false;
                    if let Some(old_value) = old_val {
                        if let Some(map) = old_value.as_object() {
                            if !map.is_empty() {
                                let dup_key = format!("{}_{}", name, next_idx);
                                parent_obj.insert(dup_key, old_value);
                                needs_advance = true;
                            }
                        }
                    }

                    let mut value = Map::with_capacity(1);
                    value.insert("#attributes".to_owned(), Value::Object(attributes));
                    parent_obj.insert(name.to_string(), Value::Object(value));

                    let _ = parent_obj;
                    let _ = parent;

                    if needs_advance {
                        self.advance_duplicate_index(name);
                    }
                }
            }
        } else {
            let parent = self.get_current_parent();
            let parent_obj = parent.as_object_mut().ok_or(SerializationError::JsonStructureError { message:
                "This is a bug - expected current value to exist, and to be an object type.\n                         Check that the value is not `Value::null`".to_string(), })?;
            parent_obj.insert(name.to_string(), Value::Null);
        }

        Ok(())
    }

    pub fn into_value(self) -> SerializationResult<Value> {
        if !self.stack.is_empty() {
            return Err(SerializationError::JsonStructureError {
                message: "Invalid stream, EOF reached before closing all attributes".to_string(),
            });
        }

        Ok(self.map)
    }
}

impl BinXmlOutput for JsonOutput {
    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
        trace!("visit_end_of_stream");
        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        trace!("visit_open_start_element: {:?}", element.name);
        let element_name = element.name.as_str();

        if element_name == "Data" {
            return self.insert_data_node(element);
        }

        // <Task>12288</Task> -> {"Task": 12288}
        if element.attributes.is_empty() {
            return self.insert_node_without_attributes(element, element_name);
        }

        self.insert_node_with_attributes(element, element_name)
    }

    fn visit_close_element(&mut self, _element: &XmlElement) -> SerializationResult<()> {
        let p = self.stack.pop();
        // Keep counters stack in sync (pop the current node's counters)
        self.dup_counters_stack.pop();
        trace!("visit_close_element: {:?}", p);
        Ok(())
    }

    fn visit_characters(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        trace!("visit_chars {:?}", &self.stack);
        // We need to clone this bool since the next statement will borrow self as mutable.
        let separate_json_attributes = self.separate_json_attributes;
        let current_value = self.get_or_create_current_path();

        // A small optimization in case we already have an owned string.
        fn value_to_json(value: Cow<BinXmlValue>) -> Value {
            if let Cow::Owned(BinXmlValue::StringType(value)) = value {
                json!(value)
            } else {
                value.into_owned().into()
            }
        }

        // If our parent is an element without any attributes,
        // we simply swap the null with the string value.
        // This is also true for the case when the attributes were inserted as our siblings.
        match current_value {
            // Regular, distinct node.
            Value::Null => {
                *current_value = value_to_json(value);
            }
            Value::Object(object) => {
                if separate_json_attributes {
                    if object.is_empty() {
                        *current_value = value_to_json(value);
                    } else {
                        // TODO: Currently we discard some of the data in this case. What should we do?
                    }
                } else {
                    // Otherwise,
                    // Should look like:
                    // ----------------
                    //  "EventID": {
                    //    "#attributes": {
                    //      "Qualifiers": ""
                    //    },
                    //    "#text": "4902"
                    //  },
                    //
                    // If multiple nodes with the same name exists, we convert the `#text` attribute into an array.
                    const TEXT_KEY: &str = "#text";
                    match object.get_mut(TEXT_KEY) {
                        // Regular, distinct node.
                        None | Some(Value::Null) => {
                            object.insert(TEXT_KEY.to_owned(), value_to_json(value));
                        }
                        // The first time we encounter another node with the same name,
                        // we convert the exiting value into an array with both values.
                        Some(Value::String(perv_value)) => {
                            let perv_value = perv_value.clone();
                            object.remove(TEXT_KEY);
                            object.insert(
                                TEXT_KEY.to_owned(),
                                json!([perv_value, value_to_json(value)]),
                            );
                        }
                        // If we already have an array, we can just push into it.
                        Some(Value::Array(arr)) => arr.push(value_to_json(value)),
                        current_value => {
                            return Err(SerializationError::JsonStructureError {
                                message: format!(
                                    "expected current value to be a String or an Array, found {current_value:?}, new value is {value:?}"
                                ),
                            });
                        }
                    }
                }
            }
            // The first time we encounter another node with the same name,
            // we convert the exiting value into an array with both values.
            Value::String(current_string) => {
                current_string.push_str(&value.as_cow_str());
            }
            // If we already have an array, we can just push into it.
            Value::Array(arr) => arr.push(value_to_json(value)),
            current_value => {
                return Err(SerializationError::JsonStructureError {
                    message: format!(
                        "expected current value to be a String or an Array, found {current_value:?}, new value is {value:?}"
                    ),
                });
            }
        }

        Ok(())
    }

    fn visit_cdata_section(&mut self) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_cdata_section", file!()),
        })
    }

    fn visit_entity_reference(&mut self, entity: &BinXmlName) -> Result<(), SerializationError> {
        // We need to create a BytesText event to access quick-xml's unescape functionality (which is private).
        // We also terminate the entity.
        let entity_ref = "&".to_string() + entity.as_str() + ";";

        let xml_event = BytesText::from_escaped(&entity_ref);
        match xml_event.unescape() {
            Ok(escaped) => {
                let as_string = escaped.to_string();

                self.visit_characters(Cow::Owned(BinXmlValue::StringType(as_string)))?;
                Ok(())
            }
            Err(_) => Err(JsonStructureError {
                message: format!("Unterminated XML Entity {}", entity_ref),
            }),
        }
    }

    fn visit_character_reference(
        &mut self,
        _char_ref: Cow<'_, str>,
    ) -> Result<(), SerializationError> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_character_reference", file!()),
        })
    }

    fn visit_processing_instruction(&mut self, _pi: &BinXmlPI) -> Result<(), SerializationError> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_processing_instruction_data", file!()),
        })
    }

    fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
        trace!("visit_start_of_stream");
        // Ensure root counters map exists
        if self.dup_counters_stack.is_empty() {
            self.dup_counters_stack
                .push(FastMap::with_hasher(ahash::RandomState::new()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::binxml::name::BinXmlName;
    use crate::binxml::value_variant::BinXmlValue;
    use crate::model::xml::{XmlAttribute, XmlElement};
    use crate::{BinXmlOutput, JsonOutput, ParserSettings};
    use pretty_assertions::assert_eq;
    use quick_xml::Reader;
    use quick_xml::events::{BytesStart, Event};
    use std::borrow::Cow;

    fn bytes_to_string(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("UTF8 Input")
    }

    fn dummy_event() -> XmlElement<'static> {
        XmlElement {
            name: Cow::Owned(BinXmlName::from_str("Dummy")),
            attributes: &[],
        }
    }

    fn event_to_element(event: BytesStart) -> XmlElement {
        let mut attrs = Vec::new();

        for attr in event.attributes() {
            let attr = attr.expect("Failed to read attribute.");
            attrs.push(XmlAttribute {
                name: Cow::Owned(BinXmlName::from_string(bytes_to_string(attr.key.as_ref()))),
                // We have to compromise here and assume all values are strings.
                value: Cow::Owned(BinXmlValue::StringType(bytes_to_string(&attr.value))),
            });
        }

        XmlElement {
            name: Cow::Owned(BinXmlName::from_string(bytes_to_string(
                event.name().as_ref(),
            ))),
            attributes: attrs.leak(),
        }
    }

    /// Converts an XML string to JSON, panics in xml is invalid.
    fn xml_to_json(xml: &str, settings: &ParserSettings) -> String {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut output = JsonOutput::new(settings);
        output.visit_start_of_stream().expect("Start of stream");

        loop {
            match reader.read_event() {
                Ok(event) => match event {
                    Event::Start(start) => {
                        output
                            .visit_open_start_element(&event_to_element(start))
                            .expect("Open start element");
                    }
                    Event::End(_) => output
                        .visit_close_element(&dummy_event())
                        .expect("Close element"),
                    Event::Empty(empty) => {
                        output
                            .visit_open_start_element(&event_to_element(empty))
                            .expect("Empty Open start element");

                        output
                            .visit_close_element(&dummy_event())
                            .expect("Empty Close");
                    }
                    Event::Text(text) => output
                        .visit_characters(Cow::Owned(BinXmlValue::StringType(bytes_to_string(
                            text.as_ref(),
                        ))))
                        .expect("Text element"),
                    Event::Comment(_) => {}
                    Event::CData(_) => unimplemented!(),
                    Event::Decl(_) => {}
                    Event::PI(_) => unimplemented!(),
                    Event::DocType(_) => {}
                    Event::Eof => {
                        output.visit_end_of_stream().expect("End of stream");
                        break;
                    }
                },
                Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            }
        }

        serde_json::to_string_pretty(&output.into_value().expect("Output")).expect("To serialize")
    }

    #[test]
    fn test_xml_to_json() {
        let s1 = r#"
<HTTPResponseHeadersInfo>
    <Header attribute1="NoProxy"></Header>
    <Header>HTTP/1.1 200 OK</Header>
</HTTPResponseHeadersInfo>
"#
        .trim();
        let s2 = r#"
{
  "HTTPResponseHeadersInfo": {
    "Header_attributes": {
      "attribute1": "NoProxy"
    },
    "Header": "HTTP/1.1 200 OK"
  }
}
"#
        .trim();

        let settings = ParserSettings::new()
            .num_threads(1)
            .separate_json_attributes(true);

        let json = xml_to_json(s1, &settings);
        println!("json: {}", json);

        assert_eq!(xml_to_json(s1, &settings), s2)
    }
}
