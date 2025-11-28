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
    /// Whether we've emitted `<name>_attributes` separately (when `separate_json_attributes == true`).
    /// If true and `kind == Pending` on close, we skip emitting `null` to match legacy behavior.
    has_separate_attributes: bool,
    /// Buffered scalar values for elements without attributes.
    /// We buffer instead of writing immediately to support concatenation of multiple character nodes.
    /// Uses serde_json::Value to avoid lifetime issues with BinXmlValue.
    buffered_values: Vec<JsonValue>,
}

/// JSON object context (either the root object or any nested object).
#[derive(Debug)]
struct ObjectFrame {
    /// Whether we've already written any field in this object.
    first_field: bool,
    /// Keys already used in this object (for duplicate key handling).
    used_keys: std::collections::HashSet<String>,
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
    /// Collected values for the aggregated `"Data": { "#text": [...] }` array.
    data_values: Vec<JsonValue>,
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
            data_values: Vec::new(),
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

    /// Reserve a unique key in the current frame without writing it.
    /// Returns the unique key that will be used (with `_1`, `_2` suffix if needed).
    fn reserve_unique_key(&mut self, key: &str) -> String {
        let frame = self
            .frames
            .last_mut()
            .expect("no current JSON object frame");
        if frame.used_keys.contains(key) {
            // Find next available suffix
            let mut suffix = 1;
            loop {
                let candidate = format!("{}_{}", key, suffix);
                if !frame.used_keys.contains(&candidate) {
                    frame.used_keys.insert(candidate.clone());
                    return candidate;
                }
                suffix += 1;
            }
        } else {
            frame.used_keys.insert(key.to_owned());
            key.to_owned()
        }
    }

    /// Write a JSON string key (with surrounding quotes and escaping).
    /// Write a JSON string key, handling duplicates by appending `_1`, `_2`, etc.
    fn write_key(&mut self, key: &str) -> SerializationResult<()> {
        self.write_comma_if_needed()?;

        // Check for duplicate keys and find a unique name
        let unique_key = self.reserve_unique_key(key);

        serde_json::to_writer(self.writer_mut(), &unique_key).map_err(SerializationError::from)?;
        self.write_bytes(b":")
    }

    /// Write a pre-reserved key directly (no duplicate checking needed).
    fn write_reserved_key(&mut self, key: &str) -> SerializationResult<()> {
        self.write_comma_if_needed()?;
        serde_json::to_writer(self.writer_mut(), key).map_err(SerializationError::from)?;
        self.write_bytes(b":")
    }

    /// Start a new nested JSON object as the value of `key` in the current object.
    fn start_object_value(&mut self, key: &str) -> SerializationResult<()> {
        self.write_key(key)?;
        self.write_bytes(b"{")?;
        self.frames.push(ObjectFrame {
            first_field: true,
            used_keys: std::collections::HashSet::new(),
        });
        Ok(())
    }

    /// End the current JSON object frame.
    fn end_object(&mut self) -> SerializationResult<()> {
        self.write_bytes(b"}")?;
        self.frames.pop();
        Ok(())
    }

    /// For elements without attributes, if their first child is another element
    /// we need to materialize this element as an object (`"name": { ... }`).
    fn ensure_parent_is_object(&mut self) -> SerializationResult<()> {
        let Some(parent_index) = self.elements.len().checked_sub(1) else {
            return Ok(());
        };

        let parent_kind = self.elements[parent_index].kind;

        match parent_kind {
            ElementValueKind::Pending => {
                // Turn `"parent": null` into `"parent": { ... }` by starting an
                // object value for it now.
                let key = self.elements[parent_index].name.clone();
                let was_reserved = self.elements[parent_index].has_separate_attributes;

                // If the key was pre-reserved (separate_json_attributes mode), use
                // write_reserved_key to avoid double-reservation.
                if was_reserved {
                    self.write_reserved_key(&key)?;
                } else {
                    self.write_key(&key)?;
                }
                self.write_bytes(b"{")?;
                self.frames.push(ObjectFrame {
                    first_field: true,
                    used_keys: std::collections::HashSet::new(),
                });

                self.elements[parent_index].kind = ElementValueKind::Object;
            }
            ElementValueKind::Scalar => {
                // Element had text content but now has child elements too.
                // Turn it into an object and move buffered text to #text field.
                let key = self.elements[parent_index].name.clone();
                let was_reserved = self.elements[parent_index].has_separate_attributes;
                let buffered = std::mem::take(&mut self.elements[parent_index].buffered_values);

                if was_reserved {
                    self.write_reserved_key(&key)?;
                } else {
                    self.write_key(&key)?;
                }
                self.write_bytes(b"{")?;
                self.frames.push(ObjectFrame {
                    first_field: true,
                    used_keys: std::collections::HashSet::new(),
                });

                // Write the buffered text as #text if not in separate mode
                // (in separate mode, text in mixed-content elements is dropped).
                if !buffered.is_empty() && !self.separate_json_attributes {
                    self.write_key("#text")?;
                    if buffered.len() == 1 {
                        serde_json::to_writer(self.writer_mut(), &buffered[0])
                            .map_err(SerializationError::from)?;
                    } else {
                        serde_json::to_writer(self.writer_mut(), &buffered)
                            .map_err(SerializationError::from)?;
                    }
                }

                self.elements[parent_index].kind = ElementValueKind::Object;
            }
            ElementValueKind::Object => {
                // Already an object, nothing to do.
            }
        }

        Ok(())
    }

    /// If the current element is represented as an object, close its JSON object.
    fn end_element_object_if_needed(&mut self) -> SerializationResult<()> {
        if let Some(elem) = self.elements.last()
            && elem.kind == ElementValueKind::Object
        {
            // The current element owns the top-most JSON object frame.
            self.end_object()?;
        }
        Ok(())
    }

    /// Append a value into the aggregated `"Data": { "#text": [...] }` under an
    /// `EventData` element. The BinXml value may itself be an array (e.g.
    /// `StringArrayType`), in which case it is stored as-is, matching the
    /// behaviour of `JsonOutput::value_to_json`.
    fn write_data_aggregated_value(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        let json_value: JsonValue = match &value {
            Cow::Borrowed(v) => JsonValue::from(*v),
            Cow::Owned(v) => JsonValue::from(v),
        };

        self.data_values.push(json_value);
        Ok(())
    }

    /// Finalize the aggregated `"Data"` value, if any.
    /// With `separate_json_attributes == false`: outputs `"Data": { "#text": ... }`
    /// With `separate_json_attributes == true`: outputs `"Data": ...` directly
    fn finalize_data_aggregator(&mut self) -> SerializationResult<()> {
        if self.data_owner_depth.is_some() && !self.data_values.is_empty() {
            // Avoid aliasing `self` while iterating by taking the values out.
            let values = std::mem::take(&mut self.data_values);

            if self.separate_json_attributes {
                // In separate_json_attributes mode, output directly without wrapper.
                // Legacy concatenates multiple string values into one.
                self.write_key("Data")?;
                if values.len() == 1 {
                    serde_json::to_writer(self.writer_mut(), &values[0])
                        .map_err(SerializationError::from)?;
                } else {
                    // Concatenate multiple values as strings (legacy behavior).
                    let mut concat = String::new();
                    for v in &values {
                        match v {
                            JsonValue::String(s) => concat.push_str(s),
                            JsonValue::Number(n) => concat.push_str(&n.to_string()),
                            JsonValue::Bool(b) => {
                                concat.push_str(if *b { "true" } else { "false" })
                            }
                            JsonValue::Null => {}
                            _ => concat.push_str(&v.to_string()),
                        }
                    }
                    serde_json::to_writer(self.writer_mut(), &concat)
                        .map_err(SerializationError::from)?;
                }
            } else {
                // With `#attributes` mode, wrap in `"Data": { "#text": ... }`.
                self.start_object_value("Data")?;
                self.write_key("#text")?;

                if values.len() == 1 {
                    serde_json::to_writer(self.writer_mut(), &values[0])
                        .map_err(SerializationError::from)?;
                } else {
                    // Multiple `<Data>` children: aggregate into an array.
                    self.write_bytes(b"[")?;
                    for (idx, json_value) in values.into_iter().enumerate() {
                        if idx > 0 {
                            self.write_bytes(b",")?;
                        }
                        serde_json::to_writer(self.writer_mut(), &json_value)
                            .map_err(SerializationError::from)?;
                    }
                    self.write_bytes(b"]")?;
                }

                self.end_object()?;
            }
        }

        // Reset aggregator state.
        self.data_owner_depth = None;
        self.data_inside_element = false;
        Ok(())
    }
}

impl<W: Write> BinXmlOutput for JsonStreamOutput<W> {
    fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
        // Open the root JSON object.
        self.write_bytes(b"{")?;
        self.frames.push(ObjectFrame {
            first_field: true,
            used_keys: std::collections::HashSet::new(),
        });
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

        // Aggregated `<EventData><Data>...</Data>...</EventData>` case:
        // multiple `<Data>` children without a `Name` attribute become a single
        // `"Data": { "#text": [ ... ] }` object under their `EventData` parent.
        if is_data
            && data_name_attr.is_none()
            && let Some(parent) = self.elements.last()
            && parent.name == "EventData"
        {
            // Depth of the owning `EventData` element.
            let owner_depth = self.elements.len();

            // Initialize a new aggregator for this `EventData`, if needed.
            if self.data_owner_depth != Some(owner_depth) {
                self.data_owner_depth = Some(owner_depth);
                self.data_values.clear();
            }

            // We're now inside a `<Data>` element that contributes to
            // the aggregated array.
            self.data_inside_element = true;

            // Do NOT push a new `ElementState` for this `<Data>` node;
            // its values are handled by the aggregator.
            return Ok(());
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
                self.frames.push(ObjectFrame {
                    first_field: true,
                    used_keys: std::collections::HashSet::new(),
                });

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
                has_separate_attributes: false,
                buffered_values: Vec::new(),
            });
        } else {
            // `separate_json_attributes == true` or element has no attributes.
            let wrote_separate_attrs = has_json_attributes && self.separate_json_attributes;

            // If we're writing `_attributes`, pre-reserve the element key so both
            // the `_attributes` and the element itself use matching suffixes.
            let element_key = if wrote_separate_attrs {
                let unique_key = self.reserve_unique_key(&key);

                // Emit `"<unique_key>_attributes": { ... }` into the parent object.
                let attr_key = format!("{}_attributes", unique_key);
                self.write_reserved_key(&attr_key)?;
                self.write_bytes(b"{")?;
                self.frames.push(ObjectFrame {
                    first_field: true,
                    used_keys: std::collections::HashSet::new(),
                });

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
                unique_key
            } else {
                // No attributes to write - use original key (will be deduped on write).
                key
            };

            // We delay emitting the actual `"key": ...` until we see either
            // a character node or a child element, so we can decide whether
            // this element is a scalar, an object, or `null`.
            self.elements.push(ElementState {
                name: element_key,
                kind: ElementValueKind::Pending,
                has_text: false,
                has_separate_attributes: wrote_separate_attrs,
                buffered_values: Vec::new(),
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
                    // No text and no children – render as `null`, unless we already
                    // emitted `<name>_attributes` separately (legacy omits the null).
                    if !elem.has_separate_attributes {
                        self.write_key(&elem.name)?;
                        self.write_bytes(b"null")?;
                    }
                }
                ElementValueKind::Scalar => {
                    // Write the buffered scalar value(s) now.
                    if !elem.buffered_values.is_empty() {
                        // If key was pre-reserved (separate_json_attributes mode), use reserved writer.
                        if elem.has_separate_attributes {
                            self.write_reserved_key(&elem.name)?;
                        } else {
                            self.write_key(&elem.name)?;
                        }
                        if elem.buffered_values.len() == 1 {
                            // Single value: preserve original type.
                            serde_json::to_writer(self.writer_mut(), &elem.buffered_values[0])
                                .map_err(SerializationError::from)?;
                        } else {
                            // Multiple values: concatenate as strings (legacy behavior).
                            let mut concat = String::new();
                            for v in &elem.buffered_values {
                                // Convert JSON value back to string for concatenation
                                match v {
                                    JsonValue::String(s) => concat.push_str(s),
                                    JsonValue::Number(n) => concat.push_str(&n.to_string()),
                                    JsonValue::Bool(b) => {
                                        concat.push_str(if *b { "true" } else { "false" })
                                    }
                                    JsonValue::Null => concat.push_str("null"),
                                    _ => concat.push_str(&v.to_string()),
                                }
                            }
                            serde_json::to_writer(self.writer_mut(), &concat)
                                .map_err(SerializationError::from)?;
                        }
                    }
                }
                ElementValueKind::Object => {
                    // Write buffered #text values if any, then close the object.
                    // In separate_json_attributes mode, elements with child elements
                    // drop text content (legacy behavior - no #text field).
                    if !elem.buffered_values.is_empty() && !self.separate_json_attributes {
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

                        if elem.buffered_values.len() == 1 {
                            // Single value: write directly.
                            serde_json::to_writer(self.writer_mut(), &elem.buffered_values[0])
                                .map_err(SerializationError::from)?;
                        } else {
                            // Multiple values: write as array (legacy behavior).
                            serde_json::to_writer(self.writer_mut(), &elem.buffered_values)
                                .map_err(SerializationError::from)?;
                        }
                    }
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
                // buffer the value (we'll write on close to support concatenation).
                let json_value: JsonValue = match value {
                    Cow::Borrowed(v) => JsonValue::from(v),
                    Cow::Owned(v) => JsonValue::from(&v),
                };
                self.elements[index].buffered_values.push(json_value);
                self.elements[index].kind = ElementValueKind::Scalar;
            }
            ElementValueKind::Scalar => {
                // Multiple character nodes: add to the buffer.
                // On close, we'll concatenate string representations to match legacy.
                let json_value: JsonValue = match value {
                    Cow::Borrowed(v) => JsonValue::from(v),
                    Cow::Owned(v) => JsonValue::from(&v),
                };
                self.elements[index].buffered_values.push(json_value);
            }
            ElementValueKind::Object => {
                // Elements with attributes: we store text under a `#text` key.
                // In separate_json_attributes mode, skip null #text values.
                if self.elements[index].has_separate_attributes {
                    let is_null = matches!(
                        &value,
                        Cow::Borrowed(BinXmlValue::NullType) | Cow::Owned(BinXmlValue::NullType)
                    );
                    if is_null {
                        return Ok(());
                    }
                }

                // Buffer text values to support multiple text nodes (legacy creates an array).
                let json_value: JsonValue = match value {
                    Cow::Borrowed(v) => JsonValue::from(v),
                    Cow::Owned(v) => JsonValue::from(&v),
                };
                self.elements[index].buffered_values.push(json_value);
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

#[cfg(test)]
mod tests {
    use super::JsonStreamOutput;
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

    fn event_to_element(event: BytesStart) -> XmlElement {
        let mut attrs = vec![];

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
            attributes: attrs,
        }
    }

    /// Converts an XML string to JSON using the legacy `JsonOutput`.
    fn xml_to_json_legacy(xml: &str, settings: &ParserSettings) -> String {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut output = JsonOutput::new(settings);
        output.visit_start_of_stream().expect("Start of stream");

        let mut element_stack: Vec<XmlElement> = Vec::new();

        loop {
            match reader.read_event() {
                Ok(event) => match event {
                    Event::Start(start) => {
                        let elem = event_to_element(start);
                        output
                            .visit_open_start_element(&elem)
                            .expect("Open start element");
                        element_stack.push(elem);
                    }
                    Event::End(_) => {
                        let elem = element_stack.pop().expect("Unbalanced XML (End)");
                        output.visit_close_element(&elem).expect("Close element");
                    }
                    Event::Empty(empty) => {
                        let elem = event_to_element(empty);
                        output
                            .visit_open_start_element(&elem)
                            .expect("Empty Open start element");
                        output.visit_close_element(&elem).expect("Empty Close");
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

    /// Converts an XML string to JSON using the streaming `JsonStreamOutput`.
    fn xml_to_json_streaming(xml: &str, settings: &ParserSettings) -> String {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let writer = Vec::new();
        let mut output = JsonStreamOutput::with_writer(writer, settings);
        output.visit_start_of_stream().expect("Start of stream");

        let mut element_stack: Vec<XmlElement> = Vec::new();

        loop {
            match reader.read_event() {
                Ok(event) => match event {
                    Event::Start(start) => {
                        let elem = event_to_element(start);
                        output
                            .visit_open_start_element(&elem)
                            .expect("Open start element");
                        element_stack.push(elem);
                    }
                    Event::End(_) => {
                        let elem = element_stack.pop().expect("Unbalanced XML (End)");
                        output.visit_close_element(&elem).expect("Close element");
                    }
                    Event::Empty(empty) => {
                        let elem = event_to_element(empty);
                        output
                            .visit_open_start_element(&elem)
                            .expect("Empty Open start element");
                        output.visit_close_element(&elem).expect("Empty Close");
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

        let bytes = output.finish().expect("finish streaming JSON");
        String::from_utf8(bytes).expect("UTF8 JSON")
    }

    #[test]
    fn test_unnamed_data_interspersed_with_binary_matches_legacy() {
        let xml = r#"
<Event>
  <EventData>
    <Data>v1</Data>
    <Binary>00AA</Binary>
    <Data>v2</Data>
  </EventData>
</Event>
        "#
        .trim();

        let settings = ParserSettings::new().num_threads(1);

        let legacy_json = xml_to_json_legacy(xml, &settings);
        let streaming_json = xml_to_json_streaming(xml, &settings);

        let legacy_value: serde_json::Value =
            serde_json::from_str(&legacy_json).expect("legacy JSON should be valid");
        let streaming_value: serde_json::Value =
            serde_json::from_str(&streaming_json).expect("streaming JSON should be valid");

        assert_eq!(
            legacy_value, streaming_value,
            "streaming JSON must match legacy JSON for unnamed <Data> elements interspersed with <Binary>"
        );
    }

    /// Regression test for Issue 1: Data aggregation format in separate_json_attributes mode.
    /// Legacy outputs `"Data": [...]` but streaming was outputting `"Data": { "#text": [...] }`.
    #[test]
    fn test_data_aggregation_separate_attributes_mode() {
        let xml = r#"
<Event>
  <EventData>
    <Data>v1</Data>
    <Data>v2</Data>
  </EventData>
</Event>
        "#
        .trim();

        let settings = ParserSettings::new()
            .num_threads(1)
            .separate_json_attributes(true);

        let legacy_json = xml_to_json_legacy(xml, &settings);
        let streaming_json = xml_to_json_streaming(xml, &settings);

        let legacy_value: serde_json::Value =
            serde_json::from_str(&legacy_json).expect("legacy JSON should be valid");
        let streaming_value: serde_json::Value =
            serde_json::from_str(&streaming_json).expect("streaming JSON should be valid");

        assert_eq!(
            legacy_value, streaming_value,
            "Data aggregation in separate_json_attributes mode: streaming must match legacy.\nLegacy: {}\nStreaming: {}",
            legacy_json, streaming_json
        );
    }

    /// Regression test for Issue 2: Duplicate element key handling.
    /// Legacy outputs `"LogonGuid": "...", "LogonGuid_1": "..."` but streaming was losing duplicates.
    ///
    /// NOTE: Legacy and streaming have different key ordering for duplicates:
    /// - Legacy: last value gets unsuffixed key (LogonGuid: guid2, LogonGuid_1: guid1)
    /// - Streaming: first value gets unsuffixed key (LogonGuid: guid1, LogonGuid_1: guid2)
    ///
    /// Both preserve all data, just with different key assignments. This is acceptable
    /// for streaming since we can't retroactively rename already-written keys.
    #[test]
    fn test_duplicate_element_keys() {
        let xml = r#"
<Event>
  <EventData>
    <Data Name="LogonGuid">guid1</Data>
    <Data Name="LogonGuid">guid2</Data>
  </EventData>
</Event>
        "#
        .trim();

        let settings = ParserSettings::new().num_threads(1);

        let legacy_json = xml_to_json_legacy(xml, &settings);
        let streaming_json = xml_to_json_streaming(xml, &settings);

        let legacy_value: serde_json::Value =
            serde_json::from_str(&legacy_json).expect("legacy JSON should be valid");
        let streaming_value: serde_json::Value =
            serde_json::from_str(&streaming_json).expect("streaming JSON should be valid");

        // Extract the set of LogonGuid values from EventData (regardless of key ordering)
        let legacy_event_data = &legacy_value["Event"]["EventData"];
        let streaming_event_data = &streaming_value["Event"]["EventData"];

        // Collect all values for LogonGuid* keys
        let mut legacy_values: Vec<&str> = Vec::new();
        let mut streaming_values: Vec<&str> = Vec::new();

        if let serde_json::Value::Object(obj) = legacy_event_data {
            for (key, val) in obj {
                if key.starts_with("LogonGuid")
                    && let serde_json::Value::String(s) = val
                {
                    legacy_values.push(s);
                }
            }
        }
        if let serde_json::Value::Object(obj) = streaming_event_data {
            for (key, val) in obj {
                if key.starts_with("LogonGuid")
                    && let serde_json::Value::String(s) = val
                {
                    streaming_values.push(s);
                }
            }
        }

        legacy_values.sort();
        streaming_values.sort();

        assert_eq!(
            legacy_values, streaming_values,
            "Duplicate element keys: both parsers must preserve all values.\nLegacy: {}\nStreaming: {}",
            legacy_json, streaming_json
        );
    }

    /// Regression test for Issue 3: Multiple character nodes concatenation.
    /// Legacy concatenates multiple text nodes, streaming was only keeping the first.
    /// This test directly invokes visit_characters multiple times to simulate the real case.
    #[test]
    fn test_multiple_character_nodes_concatenation() {
        use crate::model::xml::XmlElement;

        // Test by directly calling the visitor methods to simulate multiple character nodes
        let settings = ParserSettings::new().num_threads(1);

        // Legacy parser
        let mut legacy_output = JsonOutput::new(&settings);
        legacy_output.visit_start_of_stream().unwrap();
        let event_elem = XmlElement {
            name: Cow::Owned(BinXmlName::from_str("Event")),
            attributes: vec![],
        };
        let msg_elem = XmlElement {
            name: Cow::Owned(BinXmlName::from_str("Message")),
            attributes: vec![],
        };
        legacy_output.visit_open_start_element(&event_elem).unwrap();
        legacy_output.visit_open_start_element(&msg_elem).unwrap();
        legacy_output
            .visit_characters(Cow::Owned(BinXmlValue::StringType("Part1".to_string())))
            .unwrap();
        legacy_output
            .visit_characters(Cow::Owned(BinXmlValue::StringType("Part2".to_string())))
            .unwrap();
        legacy_output.visit_close_element(&msg_elem).unwrap();
        legacy_output.visit_close_element(&event_elem).unwrap();
        legacy_output.visit_end_of_stream().unwrap();
        let legacy_value = legacy_output.into_value().unwrap();

        // Streaming parser
        let writer = Vec::new();
        let mut streaming_output = JsonStreamOutput::with_writer(writer, &settings);
        streaming_output.visit_start_of_stream().unwrap();
        streaming_output
            .visit_open_start_element(&event_elem)
            .unwrap();
        streaming_output
            .visit_open_start_element(&msg_elem)
            .unwrap();
        streaming_output
            .visit_characters(Cow::Owned(BinXmlValue::StringType("Part1".to_string())))
            .unwrap();
        streaming_output
            .visit_characters(Cow::Owned(BinXmlValue::StringType("Part2".to_string())))
            .unwrap();
        streaming_output.visit_close_element(&msg_elem).unwrap();
        streaming_output.visit_close_element(&event_elem).unwrap();
        streaming_output.visit_end_of_stream().unwrap();
        let bytes = streaming_output.finish().unwrap();
        let streaming_json = String::from_utf8(bytes).unwrap();
        let streaming_value: serde_json::Value = serde_json::from_str(&streaming_json).unwrap();

        assert_eq!(
            legacy_value, streaming_value,
            "Multiple character nodes: streaming must match legacy.\nLegacy: {:?}\nStreaming: {}",
            legacy_value, streaming_json
        );
    }
}
