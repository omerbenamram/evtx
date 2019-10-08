use crate::err::{self, Result};
use snafu::{ensure, OptionExt, ResultExt};

use crate::binxml::value_variant::BinXmlValue;
use crate::model::xml::XmlElement;
use crate::unimplemented_fn;
use crate::xml_output::BinXmlOutput;
use crate::ParserSettings;

use core::borrow::BorrowMut;
use log::trace;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::io::Write;
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

                    mem::replace(v_temp, Value::Object(map));
                } else {
                    let current_object = v_temp
                        .as_object_mut()
                        .expect("It can only be an object or null, and null was covered");

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
    fn insert_data_node(&mut self, element: &XmlElement) -> Result<()> {
        trace!("inserting data node {:?}", &element);
        match element
            .attributes
            .iter()
            .find(|a| a.name.as_ref().0 == "Name")
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

    fn insert_node_without_attributes(&mut self, _: &XmlElement, name: &str) -> Result<()> {
        trace!("insert_node_without_attributes");
        self.stack.push(name.to_owned());

        let container =
            self.get_current_parent()
                .as_object_mut()
                .context(err::JsonStructureError {
                message:
                    "This is a bug - expected parent container to exist, and to be an object type.\
                     Check that the referencing parent is not `Value::null`",
            })?;

        container.insert(name.to_owned(), Value::Null);
        Ok(())
    }

    fn insert_node_with_attributes(&mut self, element: &XmlElement, name: &str) -> Result<()> {
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
                let value = self
                    .get_current_parent()
                    .as_object_mut()
                    .context(err::JsonStructureError {
                    message:
                        "This is a bug - expected current value to exist, and to be an object type.
                        Check that the value is not `Value::null`",
                })?;

                value.insert(format!("{}_attributes", name), Value::Object(attributes));
            } else {
                let value = self
                    .get_or_create_current_path()
                    .as_object_mut()
                    .context(err::JsonStructureError {
                    message:
                        "This is a bug - expected current value to exist, and to be an object type.
                            Check that the value is not `Value::null`",
                })?;

                value.insert("#attributes".to_owned(), Value::Object(attributes));
            }
        } else {
            // If the object does not have attributes, replace it with a null placeholder,
            // so it will be printed as a key-value pair
            let value =
                self.get_current_parent()
                    .as_object_mut()
                    .context(err::JsonStructureError {
                    message:
                        "This is a bug - expected current value to exist, and to be an object type.
                         Check that the value is not `Value::null`",
                })?;

            value.insert(name.to_string(), Value::Null);
        }

        Ok(())
    }

    pub fn into_value(self) -> Result<Value> {
        ensure!(
            self.stack.is_empty(),
            err::JsonStructureError {
                message: "Invalid stream, EOF reached before closing all attributes"
            }
        );

        Ok(self.map)
    }
}

impl BinXmlOutput for JsonOutput {
    fn visit_end_of_stream(&mut self) -> Result<()> {
        trace!("visit_end_of_stream");
        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> Result<()> {
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

    fn visit_close_element(&mut self, _element: &XmlElement) -> Result<()> {
        let p = self.stack.pop();
        trace!("visit_close_element: {:?}", p);
        Ok(())
    }

    fn visit_characters(&mut self, value: &BinXmlValue) -> Result<()> {
        trace!("visit_chars {:?}", &self.stack);
        // We need to clone this bool since the next statement will borrow self as mutable.
        let separate_json_attributes = self.separate_json_attributes;
        let current_value = self.get_or_create_current_path();

        // If our parent is an element without any attributes,
        // we simply swap the null with the string value.
        // This is also true for the case when the attributes were inserted as our siblings.
        if current_value.is_null() || separate_json_attributes {
            mem::replace(current_value, value.clone().into());
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
            let current_object =
                current_value
                    .as_object_mut()
                    .context(err::JsonStructureError {
                        message: "expected current value to be an object type",
                    })?;

            current_object.insert("#text".to_owned(), value.clone().into());
        }

        Ok(())
    }

    fn visit_cdata_section(&mut self) -> Result<()> {
        unimplemented_fn!("visit_cdata_section")
    }

    fn visit_entity_reference(&mut self) -> Result<()> {
        unimplemented_fn!("visit_entity_reference")
    }

    fn visit_processing_instruction_target(&mut self) -> Result<()> {
        unimplemented_fn!("visit_processing_instruction_target")
    }

    fn visit_processing_instruction_data(&mut self) -> Result<()> {
        unimplemented_fn!("visit_processing_instruction_data")
    }

    fn visit_start_of_stream(&mut self) -> Result<()> {
        trace!("visit_start_of_stream");
        Ok(())
    }
}
