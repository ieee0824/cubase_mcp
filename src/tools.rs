use serde_json::{Map, Value, json};

use crate::protocol::{BridgeError, ErrorCode};

// The MVP tools have no arguments yet. These variants define the extension
// point used by upcoming Track API specs and are covered by policy tests.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ArgumentType {
    String {
        min_length: Option<usize>,
        max_length: Option<usize>,
    },
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Number {
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
    Boolean,
}

impl ArgumentType {
    fn schema_type(self) -> &'static str {
        match self {
            Self::String { .. } => "string",
            Self::Integer { .. } => "integer",
            Self::Number { .. } => "number",
            Self::Boolean => "boolean",
        }
    }

    fn schema(self, description: &str) -> Value {
        let mut schema = Map::new();
        schema.insert("type".into(), Value::String(self.schema_type().into()));
        schema.insert("description".into(), Value::String(description.into()));

        match self {
            Self::String {
                min_length,
                max_length,
            } => {
                if let Some(min_length) = min_length {
                    schema.insert("minLength".into(), json!(min_length));
                }
                if let Some(max_length) = max_length {
                    schema.insert("maxLength".into(), json!(max_length));
                }
            }
            Self::Integer { minimum, maximum } => {
                if let Some(minimum) = minimum {
                    schema.insert("minimum".into(), json!(minimum));
                }
                if let Some(maximum) = maximum {
                    schema.insert("maximum".into(), json!(maximum));
                }
            }
            Self::Number { minimum, maximum } => {
                if let Some(minimum) = minimum {
                    schema.insert("minimum".into(), json!(minimum));
                }
                if let Some(maximum) = maximum {
                    schema.insert("maximum".into(), json!(maximum));
                }
            }
            Self::Boolean => {}
        }

        Value::Object(schema)
    }

    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::String {
                min_length,
                max_length,
            } => value.as_str().is_some_and(|value| {
                let length = value.chars().count();
                min_length.is_none_or(|minimum| length >= minimum)
                    && max_length.is_none_or(|maximum| length <= maximum)
            }),
            Self::Integer { minimum, maximum } => {
                let integer = if let Some(integer) = value.as_i64() {
                    Some(i128::from(integer))
                } else if let Some(integer) = value.as_u64() {
                    Some(i128::from(integer))
                } else {
                    value.as_f64().and_then(|number| {
                        (number.is_finite() && number.fract() == 0.0).then_some(number as i128)
                    })
                };

                integer.is_some_and(|integer| {
                    minimum.is_none_or(|minimum| integer >= i128::from(minimum))
                        && maximum.is_none_or(|maximum| integer <= i128::from(maximum))
                })
            }
            Self::Number { minimum, maximum } => value.as_f64().is_some_and(|number| {
                number.is_finite()
                    && minimum.is_none_or(|minimum| number >= minimum)
                    && maximum.is_none_or(|maximum| number <= maximum)
            }),
            Self::Boolean => value.is_boolean(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ArgumentSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub argument_type: ArgumentType,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub bridge_method: &'static str,
    pub read_only: bool,
    pub idempotent: bool,
    pub arguments: &'static [ArgumentSpec],
}

impl ToolSpec {
    pub fn input_schema(&self) -> Value {
        let properties = self
            .arguments
            .iter()
            .map(|argument| {
                (
                    argument.name.into(),
                    argument.argument_type.schema(argument.description),
                )
            })
            .collect::<Map<String, Value>>();
        let required = self
            .arguments
            .iter()
            .filter(|argument| argument.required)
            .map(|argument| Value::String(argument.name.into()))
            .collect::<Vec<_>>();

        let mut schema = Map::new();
        schema.insert("type".into(), Value::String("object".into()));
        schema.insert("properties".into(), Value::Object(properties));
        if !required.is_empty() {
            schema.insert("required".into(), Value::Array(required));
        }
        schema.insert("additionalProperties".into(), Value::Bool(false));
        Value::Object(schema)
    }

    pub fn mcp_definition(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema(),
            "annotations": {
                "readOnlyHint": self.read_only,
                "destructiveHint": false,
                "idempotentHint": self.idempotent,
                "openWorldHint": false
            }
        })
    }

    pub fn validate_arguments(
        &self,
        arguments: Map<String, Value>,
    ) -> Result<Map<String, Value>, BridgeError> {
        if self.arguments.is_empty() && !arguments.is_empty() {
            return Err(invalid_argument(format!(
                "Tool '{}' does not accept arguments",
                self.name
            )));
        }

        for argument in self.arguments.iter().filter(|argument| argument.required) {
            if !arguments.contains_key(argument.name) {
                return Err(invalid_argument(format!(
                    "Tool '{}' requires argument '{}'",
                    self.name, argument.name
                )));
            }
        }

        for (name, value) in &arguments {
            let Some(argument) = self.arguments.iter().find(|argument| argument.name == name)
            else {
                return Err(invalid_argument(format!(
                    "Tool '{}' does not accept argument '{name}'",
                    self.name
                )));
            };

            if !argument.argument_type.accepts(value) {
                return Err(invalid_argument(format!(
                    "Tool '{}' argument '{name}' must satisfy its JSON {} constraints",
                    self.name,
                    argument.argument_type.schema_type()
                )));
            }
        }

        Ok(arguments)
    }
}

const NO_ARGUMENTS: &[ArgumentSpec] = &[];

pub(crate) const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "cubase.get_status",
        title: "Get Cubase Status",
        description: "Get Cubase bridge connectivity and basic project/transport state.",
        bridge_method: "system.get_status",
        read_only: true,
        idempotent: true,
        arguments: NO_ARGUMENTS,
    },
    ToolSpec {
        name: "cubase.play",
        title: "Start Cubase Playback",
        description: "Start playback in the open Cubase project.",
        bridge_method: "transport.play",
        read_only: false,
        idempotent: true,
        arguments: NO_ARGUMENTS,
    },
    ToolSpec {
        name: "cubase.stop",
        title: "Stop Cubase Transport",
        description: "Stop playback or recording in Cubase.",
        bridge_method: "transport.stop",
        read_only: false,
        idempotent: true,
        arguments: NO_ARGUMENTS,
    },
    ToolSpec {
        name: "cubase.record",
        title: "Start Cubase Recording",
        description: "Start recording in the open Cubase project.",
        bridge_method: "transport.record",
        read_only: false,
        idempotent: true,
        arguments: NO_ARGUMENTS,
    },
    ToolSpec {
        name: "cubase.get_transport",
        title: "Get Cubase Transport",
        description: "Get playback, recording, tempo, and musical position when available.",
        bridge_method: "transport.get",
        read_only: true,
        idempotent: true,
        arguments: NO_ARGUMENTS,
    },
    ToolSpec {
        name: "cubase.get_capabilities",
        title: "Get Cubase Capabilities",
        description: "Get the features supported by the active Cubase bridge.",
        bridge_method: "capabilities.get",
        read_only: true,
        idempotent: true,
        arguments: NO_ARGUMENTS,
    },
];

pub(crate) fn find_tool(name: &str) -> Option<&'static ToolSpec> {
    TOOL_SPECS.iter().find(|tool| tool.name == name)
}

fn invalid_argument(message: impl Into<String>) -> BridgeError {
    BridgeError::new(ErrorCode::InvalidArgument, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGINATED_ARGUMENTS: &[ArgumentSpec] = &[
        ArgumentSpec {
            name: "cursor",
            description: "Opaque pagination cursor.",
            argument_type: ArgumentType::String {
                min_length: Some(1),
                max_length: Some(128),
            },
            required: false,
        },
        ArgumentSpec {
            name: "limit",
            description: "Maximum number of results.",
            argument_type: ArgumentType::Integer {
                minimum: Some(1),
                maximum: Some(100),
            },
            required: true,
        },
    ];

    const PAGINATED_TOOL: ToolSpec = ToolSpec {
        name: "test.paginated",
        title: "Test Paginated Tool",
        description: "Test optional and required arguments.",
        bridge_method: "transport.play",
        read_only: true,
        idempotent: true,
        arguments: PAGINATED_ARGUMENTS,
    };

    #[test]
    fn schema_and_validation_share_optional_and_required_argument_specs() {
        let schema = PAGINATED_TOOL.input_schema();
        assert_eq!(schema["properties"]["cursor"]["type"], "string");
        assert_eq!(schema["properties"]["cursor"]["minLength"], 1);
        assert_eq!(schema["properties"]["cursor"]["maxLength"], 128);
        assert_eq!(schema["properties"]["limit"]["type"], "integer");
        assert_eq!(schema["properties"]["limit"]["minimum"], 1);
        assert_eq!(schema["properties"]["limit"]["maximum"], 100);
        assert_eq!(schema["required"], json!(["limit"]));
        assert_eq!(schema["additionalProperties"], false);

        let arguments = serde_json::from_value(json!({"cursor": "next", "limit": 25})).unwrap();
        assert_eq!(
            PAGINATED_TOOL.validate_arguments(arguments).unwrap(),
            serde_json::from_value(json!({"cursor": "next", "limit": 25})).unwrap()
        );
    }

    #[test]
    fn optional_arguments_may_be_omitted() {
        let arguments = serde_json::from_value(json!({"limit": 25})).unwrap();
        assert!(PAGINATED_TOOL.validate_arguments(arguments).is_ok());
    }

    #[test]
    fn integer_arguments_follow_json_schema_numeric_semantics() {
        for limit in [json!(25), json!(25.0), json!(1e2)] {
            let arguments = serde_json::from_value(json!({"limit": limit})).unwrap();
            assert!(PAGINATED_TOOL.validate_arguments(arguments).is_ok());
        }
    }

    #[test]
    fn integer_bounds_are_exact_above_the_f64_safe_integer_range() {
        let exact_i64_max = ArgumentType::Integer {
            minimum: Some(i64::MAX),
            maximum: Some(i64::MAX),
        };

        assert!(exact_i64_max.accepts(&json!(i64::MAX)));
        assert!(!exact_i64_max.accepts(&json!(i64::MAX as u64 + 1)));
        assert!(!exact_i64_max.accepts(&json!(i64::MAX as f64)));
    }

    #[test]
    fn required_unknown_and_wrong_type_arguments_are_rejected() {
        for arguments in [
            json!({"cursor": "next"}),
            json!({"limit": 25, "unexpected": true}),
            json!({"limit": "25"}),
            json!({"limit": []}),
            json!({"limit": {"nested": true}}),
            json!({"limit": null}),
            json!({"cursor": "", "limit": 25}),
            json!({"cursor": "x".repeat(129), "limit": 25}),
            json!({"limit": 0}),
            json!({"limit": 101}),
        ] {
            let arguments = serde_json::from_value(arguments).unwrap();
            let error = PAGINATED_TOOL.validate_arguments(arguments).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument);
        }
    }

    #[test]
    fn all_supported_argument_types_reject_mismatched_json_values() {
        let cases = [
            (
                ArgumentType::String {
                    min_length: None,
                    max_length: None,
                },
                json!(false),
            ),
            (
                ArgumentType::Integer {
                    minimum: None,
                    maximum: None,
                },
                json!(1.5),
            ),
            (
                ArgumentType::Number {
                    minimum: None,
                    maximum: None,
                },
                json!("1.5"),
            ),
            (ArgumentType::Boolean, json!(0)),
        ];

        for (argument_type, value) in cases {
            assert!(!argument_type.accepts(&value));
        }
    }

    #[test]
    fn catalog_names_and_argument_names_are_unique_and_nonempty() {
        let mut tool_names = std::collections::HashSet::new();

        for tool in TOOL_SPECS {
            assert!(!tool.name.is_empty());
            assert!(
                tool_names.insert(tool.name),
                "duplicate tool: {}",
                tool.name
            );

            let mut argument_names = std::collections::HashSet::new();
            for argument in tool.arguments {
                assert!(!argument.name.is_empty());
                assert!(
                    argument_names.insert(argument.name),
                    "duplicate argument '{}' on tool '{}'",
                    argument.name,
                    tool.name
                );
            }
        }
    }
}
