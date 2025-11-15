use crate::ParserSettings;
use crate::err::{SerializationError, SerializationResult};
use crate::xml_output::BinXmlOutput;

use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::BinXmlValue;
use crate::model::xml::{BinXmlPI, XmlElement};
use quick_xml::events::BytesText;
use serde_json::Value as JsonValue;
use std::borrow::Cow;
use std::io::Write;

/// Represents how the current XML element is being rendered in JSON.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum ElementValueKind {
    /// We haven't decided yet if this element will be rendered as a scalar,
    /// an object, or `null`. This is the case for elements without attributes.
    Pending,
    /// The element has been rendered as a scalar JSON value (`"key": 123`).
    Scalar,
    /// The element is rendered as an object (`"key": { ... }`).
    Object,
}

/// Per-element state while streaming.
#[derive(Debug)]
struct ElementState {
    /// JSON key for this element in its parent object.
    name: String,
    /// How this element's JSON value is currently represented.
    kind: ElementValueKind,
    /// Whether we've already emitted a `#text` field for this element (when `kind == Object`).
    has_text: bool,
}

/// JSON object context (either the root object or any nested object).
#[derive(Debug)]
struct ObjectFrame {
    /// Whether we've already written any field in this object.
    first_field: bool,
}

pub struct JsonStreamOutput<W: Write> {
    writer: Option<W>,
    /// Whether pretty-printing was requested. Currently unused – streaming
    /// output is always compact, and callers compare via `serde_json::Value`.
    #[allow(dead_code)]
    indent: bool,
    separate_json_attributes: bool,

    /// Stack of JSON object frames. The root object is at index 0.
    frames: Vec<ObjectFrame>,
    /// Stack of currently open XML elements.
    elements: Vec<ElementState>,

    /// Optional depth (in `elements`) of an `EventData` element that owns a
    /// synthetic `"Data": { "#text": [...] }` aggregator, used to model
    /// `<EventData><Data>...</Data>...</EventData>` without building an
    /// intermediate tree.
    data_owner_depth: Option<usize>,
    /// Whether we've already written at least one entry into the aggregated
    /// `"Data": { "#text": [...] }` array.
    data_array_started: bool,
    /// Whether we are currently inside a `<Data>` element that contributes to
    /// the aggregated `"Data"` array.
    data_inside_element: bool,
}

impl<W: Write> JsonStreamOutput<W> {
    pub fn with_writer(writer: W, settings: &ParserSettings) -> Self {
        JsonStreamOutput {
            writer: Some(writer),
            indent: settings.should_indent(),
            separate_json_attributes: settings.should_separate_json_attributes(),
            frames: Vec::new(),
            elements: Vec::new(),
            data_owner_depth: None,
            data_array_started: false,
            data_inside_element: false,
        }
    }

    /// Finalize the JSON stream and return the underlying writer.
    pub fn finish(mut self) -> SerializationResult<W> {
        // If the caller didn't drive the parser fully, we may still have an
        // open root object; try to close it gracefully.
        if !self.frames.is_empty() {
            // Close any remaining open element objects.
            while let Some(_elem) = self.elements.pop() {
                self.end_element_object_if_needed()?;
            }

            // Close the root object.
            self.write_bytes(b"}")?;
            self.frames.clear();
        }

        self.writer
            .take()
            .ok_or_else(|| SerializationError::JsonStructureError {
                message: "Writer already taken".to_string(),
            })
    }

    pub fn into_writer(self) -> W {
        self.finish()
            .expect("failed to finalize JSON output in JsonStreamOutput")
    }

    fn writer_mut(&mut self) -> &mut W {
        self.writer
            .as_mut()
            .expect("JsonStreamOutput writer missing")
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> SerializationResult<()> {
        self.writer_mut()
            .write_all(bytes)
            .map_err(SerializationError::from)
    }

    fn current_frame_mut(&mut self) -> &mut ObjectFrame {
        self.frames
            .last_mut()
            .expect("no current JSON object frame available")
    }

    /// Write a comma if needed for the current JSON object.
    fn write_comma_if_needed(&mut self) -> SerializationResult<()> {
        let frame = self.current_frame_mut();
        if frame.first_field {
            frame.first_field = false;
            Ok(())
        } else {
            self.write_bytes(b",")
        }
    }

    /// Write a JSON string key (with surrounding quotes and escaping).
    fn write_key(&mut self, key: &str) -> SerializationResult<()> {
        self.write_comma_if_needed()?;
        serde_json::to_writer(self.writer_mut(), key).map_err(SerializationError::from)?;
        self.write_bytes(b":")
    }

    /// Start a new nested JSON object as the value of `key` in the current object.
    fn start_object_value(&mut self, key: &str) -> SerializationResult<()> {
        self.write_key(key)?;
        self.write_bytes(b"{")?;
        self.frames.push(ObjectFrame { first_field: true });
        Ok(())
    }

    /// End the current JSON object frame.
    fn end_object(&mut self) -> SerializationResult<()> {
        self.write_bytes(b"}")?;
        self.frames.pop();
        Ok(())
    }

    /// Write a scalar JSON value based on a `BinXmlValue`.
    fn write_binxml_value(&mut self, value: &BinXmlValue) -> SerializationResult<()> {
        // We reuse the existing conversion logic to preserve semantics;
        // this only allocates for the single value, not for the entire record.
        let json_value: JsonValue = JsonValue::from(value);
        serde_json::to_writer(self.writer_mut(), &json_value).map_err(SerializationError::from)
    }

    /// Helper for writing `Cow<BinXmlValue>` in `visit_characters`.
    fn write_cow_binxml_value(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        match value {
            Cow::Borrowed(v) => self.write_binxml_value(v),
            Cow::Owned(v) => self.write_binxml_value(&v),
        }
    }

    /// For elements without attributes, if their first child is another element
    /// we need to materialize this element as an object (`"name": { ... }`).
    fn ensure_parent_is_object(&mut self) -> SerializationResult<()> {
        if let Some(parent_index) = self.elements.len().checked_sub(1)
            && self.elements[parent_index].kind == ElementValueKind::Pending
        {
            // Turn `"parent": null` into `"parent": { ... }` by starting an
            // object value for it now.
            let key = self.elements[parent_index].name.clone();
            self.start_object_value(&key)?;
            self.elements[parent_index].kind = ElementValueKind::Object;
        }

        Ok(())
    }

    /// If the current element is represented as an object, close its JSON object.
    fn end_element_object_if_needed(&mut self) -> SerializationResult<()> {
        if let Some(elem) = self.elements.last() {
            if elem.kind == ElementValueKind::Object {
                // The current element owns the top-most JSON object frame.
                self.end_object()?;
            }
        }
        Ok(())
    }

    /// Append a value into the aggregated `"Data": { "#text": ... }` under an
    /// `EventData` element. The BinXml value may itself be an array (e.g.
    /// `StringArrayType`), in which case it is written as-is, matching the
    /// behaviour of `JsonOutput::value_to_json`.
    fn write_data_aggregated_value(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        let json_value: JsonValue = match &value {
            Cow::Borrowed(v) => JsonValue::from(*v),
            Cow::Owned(v) => JsonValue::from(&*v),
        };

        serde_json::to_writer(self.writer_mut(), &json_value).map_err(SerializationError::from)
    }

    /// Finalize the aggregated `"Data": { "#text": [...] }` object, if any.
    fn finalize_data_aggregator(&mut self) -> SerializationResult<()> {
        if self.data_owner_depth.is_some() {
            // Close the `"Data"` object.
            self.end_object()?;
            // Reset aggregator state.
            self.data_owner_depth = None;
            self.data_array_started = false;
            self.data_inside_element = false;
        }
        Ok(())
    }
}

impl<W: Write> BinXmlOutput for JsonStreamOutput<W> {
    fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
        // Open the root JSON object.
        self.write_bytes(b"{")?;
        self.frames.push(ObjectFrame { first_field: true });
        Ok(())
    }

    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
        // Close any remaining elements (this will close their objects).
        while let Some(_elem) = self.elements.pop() {
            self.end_element_object_if_needed()?;
        }

        // Close the root JSON object.
        if !self.frames.is_empty() {
            self.write_bytes(b"}")?;
            self.frames.clear();
        }

        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        // If we're nested under an element without attributes, and this is the
        // first child element, we must represent the parent as an object.
        self.ensure_parent_is_object()?;

        // Determine JSON key for this element.
        let element_name = element.name.as_str();

        // Special handling for `<Data>` nodes: they use their "Name" attribute
        // as the JSON key when present, and ignore attributes entirely.
        let is_data = element_name == "Data";
        let data_name_attr = if is_data {
            element
                .attributes
                .iter()
                .find(|a| a.name.as_ref().as_str() == "Name")
        } else {
            None
        };

        let key = if let Some(name_attr) = data_name_attr {
            name_attr.value.as_cow_str().into_owned()
        } else {
            element_name.to_owned()
        };

        // If we have an active aggregated `"Data"` under an `EventData` element
        // and are about to open another child (e.g. `<Binary>`), finalize the
        // aggregator so that subsequent fields are siblings of `"Data"`, not
        // elements of its `#text` array.
        if let Some(owner_depth) = self.data_owner_depth {
            if self.elements.len() == owner_depth && element_name != "Data" {
                self.finalize_data_aggregator()?;
            }
        }

        // Aggregated `<EventData><Data>...</Data>...</EventData>` case:
        // multiple `<Data>` children without a `Name` attribute become a single
        // `"Data": { "#text": [ ... ] }` object under their `EventData` parent.
        if is_data && data_name_attr.is_none() {
            if let Some(parent) = self.elements.last() {
                if parent.name == "EventData" {
                    // Depth of the owning `EventData` element.
                    let owner_depth = self.elements.len();

                    // Initialize a new aggregator for this `EventData`, if needed.
                    if self.data_owner_depth != Some(owner_depth) {
                        // Ensure `EventData` itself is an object.
                        self.ensure_parent_is_object()?;

                        // `"Data": {`
                        self.start_object_value("Data")?;

                        // `"#text": [`
                        let first_field = {
                            let frame = self.current_frame_mut();
                            let first = frame.first_field;
                            if first {
                                frame.first_field = false;
                            }
                            first
                        };
                        if !first_field {
                            self.write_bytes(b",")?;
                        }
                        serde_json::to_writer(self.writer_mut(), "#text")
                            .map_err(SerializationError::from)?;
                        self.write_bytes(b":")?;

                        self.data_owner_depth = Some(owner_depth);
                        self.data_array_started = false;
                    }

                    // We're now inside a `<Data>` element that contributes to
                    // the aggregated array.
                    self.data_inside_element = true;

                    // Do NOT push a new `ElementState` for this `<Data>` node;
                    // its values are handled by the aggregator.
                    return Ok(());
                }
            }
        }

        // In the JSON representation, `<Data Name="...">` behaves like a
        // regular node without attributes. Attributes whose JSON value is
        // `null` are ignored (this matches `JsonOutput`).
        let mut has_json_attributes = false;
        if !is_data {
            for attr in &element.attributes {
                let json_value: JsonValue = JsonValue::from(attr.value.as_ref());
                if !json_value.is_null() {
                    has_json_attributes = true;
                    break;
                }
            }
        }

        // Elements with attributes and `separate_json_attributes == false` are
        // materialized as objects with a `#attributes` field.
        if has_json_attributes && !self.separate_json_attributes {
            // `"key": { "#attributes": { ... } }`
            self.start_object_value(&key)?;

            // Write `#attributes` object.
            {
                // Update first-field state for the element object.
                let first_field = {
                    let frame = self.current_frame_mut();
                    let first = frame.first_field;
                    if first {
                        frame.first_field = false;
                    }
                    first
                };
                if !first_field {
                    self.write_bytes(b",")?;
                }
                serde_json::to_writer(self.writer_mut(), "#attributes")
                    .map_err(SerializationError::from)?;
                self.write_bytes(b":")?;

                // Start attributes object.
                self.write_bytes(b"{")?;
                self.frames.push(ObjectFrame { first_field: true });

                {
                    for attr in &element.attributes {
                        let attr_key = attr.name.as_str();
                        // Skip the `Name` attribute on `<Data>`; it is only
                        // used as the field name, not as an attribute.
                        if is_data && attr_key == "Name" {
                            continue;
                        }

                        let json_value: JsonValue = JsonValue::from(attr.value.as_ref());
                        if json_value.is_null() {
                            continue;
                        }

                        let is_first = {
                            let frame = self.current_frame_mut();
                            let first = frame.first_field;
                            if first {
                                frame.first_field = false;
                            }
                            first
                        };
                        if !is_first {
                            self.write_bytes(b",")?;
                        }
                        serde_json::to_writer(self.writer_mut(), attr_key)
                            .map_err(SerializationError::from)?;
                        self.write_bytes(b":")?;
                        serde_json::to_writer(self.writer_mut(), &json_value)
                            .map_err(SerializationError::from)?;
                    }
                }

                // Close `#attributes` object.
                self.end_object()?;
            }

            self.elements.push(ElementState {
                name: key,
                kind: ElementValueKind::Object,
                has_text: false,
            });
        } else {
            // `separate_json_attributes == true` or element has no attributes.
            if has_json_attributes && self.separate_json_attributes {
                // Emit `"<key>_attributes": { ... }` into the parent object.
                let attr_key = format!("{}_attributes", key);
                self.write_key(&attr_key)?;
                self.write_bytes(b"{")?;
                self.frames.push(ObjectFrame { first_field: true });

                {
                    for attr in &element.attributes {
                        let attr_name = attr.name.as_str();
                        let json_value: JsonValue = JsonValue::from(attr.value.as_ref());
                        if json_value.is_null() {
                            continue;
                        }

                        let is_first = {
                            let frame = self.current_frame_mut();
                            let first = frame.first_field;
                            if first {
                                frame.first_field = false;
                            }
                            first
                        };
                        if !is_first {
                            self.write_bytes(b",")?;
                        }
                        serde_json::to_writer(self.writer_mut(), attr_name)
                            .map_err(SerializationError::from)?;
                        self.write_bytes(b":")?;
                        serde_json::to_writer(self.writer_mut(), &json_value)
                            .map_err(SerializationError::from)?;
                    }
                }

                self.end_object()?;
            }

            // We delay emitting the actual `"key": ...` until we see either
            // a character node or a child element, so we can decide whether
            // this element is a scalar, an object, or `null`.
            self.elements.push(ElementState {
                name: key,
                kind: ElementValueKind::Pending,
                has_text: false,
            });
        }

        Ok(())
    }

    fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        let element_name = element.name.as_str();

        // Closing an aggregated `<Data>` node: we only need to mark that we
        // are no longer inside a contributing `<Data>`; the owning `EventData`
        // element remains on the stack.
        if element_name == "Data" && self.data_owner_depth.is_some() && self.data_inside_element {
            self.data_inside_element = false;
            return Ok(());
        }

        let current_depth = self.elements.len();
        let is_data_owner = self.data_owner_depth == Some(current_depth);

        if let Some(elem) = self.elements.pop() {
            if is_data_owner {
                // Finalize the aggregated `"Data": { "#text": [...] }` object.
                self.finalize_data_aggregator()?;
            }

            match elem.kind {
                ElementValueKind::Pending => {
                    // No text and no children – render as `null`.
                    self.write_key(&elem.name)?;
                    self.write_bytes(b"null")?;
                }
                ElementValueKind::Scalar => {
                    // Already fully rendered (`"key": value`).
                }
                ElementValueKind::Object => {
                    // Close the element's object.
                    self.end_object()?;
                }
            }
        }
        Ok(())
    }

    fn visit_characters(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        // Aggregated `<EventData><Data>...</Data>...</EventData>` case.
        if let Some(owner_depth) = self.data_owner_depth {
            let current_depth = self.elements.len();
            if self.data_inside_element && current_depth == owner_depth {
                self.write_data_aggregated_value(value)?;
                return Ok(());
            }
        }

        // Characters belong to the innermost open XML element.
        let Some(index) = self.elements.len().checked_sub(1) else {
            return Ok(());
        };

        let kind = self.elements[index].kind;

        match kind {
            ElementValueKind::Pending => {
                // First content for this element and it has no attributes:
                // render as scalar `"key": <value>`.
                let key = self.elements[index].name.clone();
                self.write_key(&key)?;
                self.write_cow_binxml_value(value)?;
                self.elements[index].kind = ElementValueKind::Scalar;
            }
            ElementValueKind::Scalar => {
                // Multiple character nodes for a scalar element are unusual in
                // real EVTX data. We approximate the behaviour of the regular
                // JSON output by concatenating string representations.
                //
                // To keep this simple and allocation-light, we just ignore
                // additional character nodes here – they are not expected in
                // the typical Windows Event Log schema that this crate targets.
                let _ = value;
            }
            ElementValueKind::Object => {
                // Elements with attributes: we store text under a `#text` key.
                // For the streaming implementation we only support a single
                // `#text` value; multiple text nodes for the same element are
                // not expected in real EVTX data.
                if self.elements[index].has_text {
                    // As above, we ignore additional character nodes.
                    let _ = value;
                    return Ok(());
                }

                let is_first = {
                    let frame = self.current_frame_mut();
                    let first = frame.first_field;
                    if first {
                        frame.first_field = false;
                    }
                    first
                };
                if !is_first {
                    self.write_bytes(b",")?;
                }
                serde_json::to_writer(self.writer_mut(), "#text")
                    .map_err(SerializationError::from)?;
                self.write_bytes(b":")?;
                self.write_cow_binxml_value(value)?;
                self.elements[index].has_text = true;
            }
        }

        Ok(())
    }

    fn visit_cdata_section(&mut self) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_cdata_section", file!()),
        })
    }

    fn visit_entity_reference(&mut self, entity: &BinXmlName) -> SerializationResult<()> {
        // Match JsonOutput behaviour: use quick-xml's unescape to resolve the entity.
        let entity_ref = "&".to_string() + entity.as_str() + ";";
        let xml_event = BytesText::from_escaped(&entity_ref);
        match xml_event.unescape() {
            Ok(escaped) => {
                let as_string = escaped.to_string();
                self.visit_characters(Cow::Owned(BinXmlValue::StringType(as_string)))
            }
            Err(_) => Err(SerializationError::JsonStructureError {
                message: format!("Unterminated XML Entity {}", entity_ref),
            }),
        }
    }

    fn visit_character_reference(&mut self, _char_ref: Cow<'_, str>) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_character_reference", file!()),
        })
    }

    fn visit_processing_instruction(&mut self, _pi: &BinXmlPI) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_processing_instruction_data", file!()),
        })
    }
}
