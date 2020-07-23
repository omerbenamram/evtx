use crate::err::{SerializationError, SerializationResult};

use crate::binxml::value_variant::BinXmlValue;
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::xml_output::BinXmlOutput;
use crate::ParserSettings;

use core::borrow::BorrowMut;
use log::trace;
use serde_json::{Map, Value};
use std::borrow::Cow;

use crate::binxml::name::BinXmlName;
use crate::err::SerializationError::JsonStructureError;
use quick_xml::events::BytesText;
use std::mem;

pub struct JsonOutput {
    map: Value,
    stack: Vec<String>,
    separate_json_attributes: bool,
}

impl JsonOutput {
    pub fn new(settings: &ParserSettings) -> Self {
        JsonOutput {
            map: Value::Object(Map::new()),
            stack: vec![],
            separate_json_attributes: settings.should_separate_json_attributes(),
        }
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
                    let mut map = Map::new();
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
                    let mut map = Map::new();
                    map.insert(key.clone(), v_temp.clone());

                    *v_temp = Value::Object(map);
                } else {
                    let current_object = v_temp
                        .as_object_mut()
                        .expect("!v_temp.is_object was matched above.");

                    current_object.insert(key.clone(), Value::Object(Map::new()));
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

        let container = self.get_current_parent().as_object_mut().ok_or_else(|| {
            SerializationError::JsonStructureError {
                message:
                    "This is a bug - expected parent container to exist, and to be an object type.\
                     Check that the referencing parent is not `Value::null`"
                        .to_string(),
            }
        })?;

        // We do a linear probe in case XML contains duplicate keys like so:
        //    <HTTPResponseHeadersInfo>
        //        <Header>HTTP/1.1 200 OK</Header>
        //        <Header>x-ms-version: 2009-09-19</Header>
        //        <Header>x-ms-lease-status: unlocked</Header>
        //        <Header>x-ms-blob-type: BlockBlob</Header>
        //    </HTTPResponseHeadersInfo>
        //
        // Insertions should look like:
        //
        //    {"Header": Object({})}
        //    {"Header": String("HTTP/1.1 200 OK")}
        //    {"Header": String("x-ms-version: 2009-09-19"), "Header_1": String("HTTP/1.1 200 OK")}
        //   ....
        //
        if let Some(old_value) = container.insert(name.to_string(), Value::Null) {
            // Value should move to next slot, key should remain free to allow for next value.

            // If old value is a placeholder, we don't yet move it, to avoid creating empty placeholers.
            // A placeholder can be either a `Null` or an empty Map.
            if old_value.is_null() {
                return Ok(());
            }

            if let Some(map) = old_value.as_object() {
                if map.is_empty() {
                    return Ok(());
                }
            }

            let mut free_slot = 1;

            // If it is a concrete value, we look for another slot.
            while container.get(&format!("{}_{}", name, free_slot)).is_some() {
                // Value is an empty object - we can override it's value.
                free_slot += 1
            }

            container.insert(format!("{}_{}", name, free_slot), old_value);
        };

        Ok(())
    }

    fn insert_node_with_attributes(
        &mut self,
        element: &XmlElement,
        name: &str,
    ) -> SerializationResult<()> {
        trace!("insert_node_with_attributes");
        self.stack.push(name.to_owned());

        let mut attributes = Map::new();

        for attribute in element.attributes.iter() {
            let value = attribute.value.clone().into_owned();
            let value: Value = value.into();

            if !value.is_null() {
                let name: &str = attribute.name.as_str();
                attributes.insert(name.to_owned(), value);
            }
        }

        // If we have attributes, create a map as usual.
        if !attributes.is_empty() {
            if self.separate_json_attributes {
                // If we are separating the attributes we want
                // to insert the object for the attributes
                // into the parent.
                let value = self.get_current_parent().as_object_mut().ok_or_else(|| {
                    SerializationError::JsonStructureError {
                    message:
                        "This is a bug - expected current value to exist, and to be an object type.
                        Check that the value is not `Value::null`"
                            .to_string(),
                }
                })?;

                value.insert(format!("{}_attributes", name), Value::Object(attributes));

                // If the element's main value is empty, we want to remove it because we
                // do not want the value to represent an empty object.
                if value[name] == Value::Object(Map::new()) {
                    value.remove(name);
                }
            } else {
                let value = self
                    .get_or_create_current_path()
                    .as_object_mut()
                    .ok_or_else(|| {
                        SerializationError::JsonStructureError {
                    message:
                        "This is a bug - expected current value to exist, and to be an object type.
                            Check that the value is not `Value::null`"
                            .to_string(),
                }
                    })?;

                value.insert("#attributes".to_owned(), Value::Object(attributes));
            }
        } else {
            // If the object does not have attributes, replace it with a null placeholder,
            // so it will be printed as a key-value pair
            let value =
                self.get_current_parent()
                    .as_object_mut()
                    .ok_or(SerializationError::JsonStructureError {
                    message:
                        "This is a bug - expected current value to exist, and to be an object type.
                         Check that the value is not `Value::null`"
                            .to_string(),
                })?;

            value.insert(name.to_string(), Value::Null);
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
        trace!("visit_close_element: {:?}", p);
        Ok(())
    }

    fn visit_characters(&mut self, value: &BinXmlValue) -> SerializationResult<()> {
        trace!("visit_chars {:?}", &self.stack);
        // We need to clone this bool since the next statement will borrow self as mutable.
        let separate_json_attributes = self.separate_json_attributes;
        let current_value = self.get_or_create_current_path();

        // If our parent is an element without any attributes,
        // we simply swap the null with the string value.
        // This is also true for the case when the attributes were inserted as our siblings.
        if current_value.is_null() || separate_json_attributes {
            *current_value = value.clone().into();
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
            if let Some(object) = current_value.as_object_mut() {
                object.insert("#text".to_owned(), value.clone().into());
                return Ok(());
            };

            // If we already have a string (because we got two consecutive `Character` events,
            // Concat them.
            if let Some(s) = current_value.as_str() {
                let new_string = s.to_string();
                mem::replace(
                    current_value,
                    Value::String(new_string + &value.as_cow_str()),
                )
            } else {
                return Err(SerializationError::JsonStructureError {
                    message: format!(
                        "expected current value to be an object type or a String, found {:?}, value is {:?}",
                        current_value, value
                    ),
                });
            };
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

        let xml_event = BytesText::from_escaped_str(&entity_ref);
        match xml_event.unescaped() {
            Ok(escaped) => {
                let as_string = String::from_utf8(escaped.to_vec())
                    .expect("This cannot fail, since it was a valid string beforehand");

                self.visit_characters(&BinXmlValue::StringType(as_string))?;
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
    use quick_xml::events::{BytesStart, Event};
    use quick_xml::Reader;
    use std::borrow::Cow;

    fn bytes_to_string(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("UTF8 Input")
    }

    fn dummy_event() -> XmlElement<'static> {
        XmlElement {
            name: Cow::Owned(BinXmlName::from_str("Dummy")),
            attributes: vec![],
        }
    }

    fn event_to_element(event: BytesStart) -> XmlElement {
        let mut attrs = vec![];

        for attr in event.attributes() {
            let attr = attr.expect("Failed to read attribute.");
            attrs.push(XmlAttribute {
                name: Cow::Owned(BinXmlName::from_string(bytes_to_string(attr.key))),
                // We have to compromise here and assume all values are strings.
                value: Cow::Owned(BinXmlValue::StringType(bytes_to_string(&attr.value))),
            });
        }

        XmlElement {
            name: Cow::Owned(BinXmlName::from_string(bytes_to_string(event.name()))),
            attributes: attrs,
        }
    }

    /// Converts an XML string to JSON, panics in xml is invalid.
    fn xml_to_json(xml: &str, settings: &ParserSettings) -> String {
        let mut reader = Reader::from_str(xml);
        reader.trim_text(true);

        let mut output = JsonOutput::new(settings);
        output.visit_start_of_stream().expect("Start of stream");

        let mut buf = vec![];

        loop {
            match reader.read_event(&mut buf) {
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
                        .visit_characters(&BinXmlValue::StringType(bytes_to_string(text.as_ref())))
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
    "Header": "HTTP/1.1 200 OK",
    "Header_attributes": {
      "attribute1": "NoProxy"
    }
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
