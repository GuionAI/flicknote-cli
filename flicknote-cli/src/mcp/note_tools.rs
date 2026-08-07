use flicknote_core::services::dto::ExtractionFilterDto;
use flicknote_core::services::source::SourceView;
use rmcp::schemars::JsonSchema;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;

const INVALID_NOTE_ID: &str = "invalid note ID";

fn invalid_note_id<E: de::Error>() -> Result<i64, E> {
    Err(E::custom(INVALID_NOTE_ID))
}

fn deserialize_note_id<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    struct NoteIdVisitor;

    impl<'de> Visitor<'de> for NoteIdVisitor {
        type Value = i64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an integer or a digit-only decimal note ID")
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            i64::try_from(value).map_err(|_| E::custom(INVALID_NOTE_ID))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return invalid_note_id();
            }
            value.parse::<i64>().map_err(|_| E::custom(INVALID_NOTE_ID))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }

        fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            invalid_note_id()
        }

        fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            invalid_note_id()
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            invalid_note_id()
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            invalid_note_id()
        }

        fn visit_seq<A>(self, _: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            invalid_note_id()
        }

        fn visit_map<A>(self, _: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            invalid_note_id()
        }
    }

    deserializer.deserialize_any(NoteIdVisitor)
}

fn default_limit() -> u32 {
    20
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "NoteType")]
pub(super) enum ListNoteType {
    Normal,
    Meeting,
    Link,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "NoteType")]
pub(super) enum CountNoteType {
    Normal,
    Meeting,
    Link,
    File,
}

impl ListNoteType {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Meeting => "meeting",
            Self::Link => "link",
        }
    }
}

impl CountNoteType {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Meeting => "meeting",
            Self::Link => "link",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteListParams {
    #[serde(rename = "type")]
    pub note_type: Option<ListNoteType>,
    pub project: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteFindParams {
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub extractions: Vec<ExtractionFilterDto>,
    pub project: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteCountParams {
    #[serde(default)]
    pub keywords: Vec<String>,
    pub project: Option<String>,
    #[serde(rename = "type")]
    pub note_type: Option<CountNoteType>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteIdParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteGetParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteSectionParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
    pub section: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteAddParams {
    pub content: String,
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteModifyParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
    pub before: Option<String>,
    pub after: Option<String>,
    pub section: Option<String>,
    pub project: Option<String>,
    pub flagged: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteContentParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteInsertParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
    pub section: String,
    pub position: flicknote_core::services::dto::InsertPosition,
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteSectionContentParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
    pub section: String,
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteRenameSectionParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
    pub section: String,
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteSourceParams {
    #[serde(deserialize_with = "deserialize_note_id")]
    pub id: i64,
    #[serde(default)]
    pub archived: bool,
    pub range: Option<String>,
    #[serde(default)]
    pub view: SourceView,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_note_id_parameter_structs_accept_digit_only_string_ids() {
        assert_eq!(
            serde_json::from_value::<NoteIdParams>(json!({ "id": "42" }))
                .unwrap()
                .id,
            42
        );
        assert_eq!(
            serde_json::from_value::<NoteGetParams>(json!({ "id": "42" }))
                .unwrap()
                .id,
            42
        );
        assert_eq!(
            serde_json::from_value::<NoteSectionParams>(json!({ "id": "42", "section": "s" }))
                .unwrap()
                .id,
            42
        );
        assert_eq!(
            serde_json::from_value::<NoteModifyParams>(json!({ "id": "42" }))
                .unwrap()
                .id,
            42
        );
        assert_eq!(
            serde_json::from_value::<NoteContentParams>(
                json!({ "id": "42", "content": "content" })
            )
            .unwrap()
            .id,
            42
        );
        assert_eq!(
            serde_json::from_value::<NoteInsertParams>(json!({
                "id": "42",
                "section": "s",
                "position": "before",
                "content": "content"
            }))
            .unwrap()
            .id,
            42
        );
        assert_eq!(
            serde_json::from_value::<NoteSectionContentParams>(json!({
                "id": "42",
                "section": "s",
                "content": "content"
            }))
            .unwrap()
            .id,
            42
        );
        assert_eq!(
            serde_json::from_value::<NoteRenameSectionParams>(json!({
                "id": "42",
                "section": "s",
                "name": "name"
            }))
            .unwrap()
            .id,
            42
        );
        assert_eq!(
            serde_json::from_value::<NoteSourceParams>(json!({ "id": "42" }))
                .unwrap()
                .id,
            42
        );
    }

    #[test]
    fn note_id_strings_reject_non_decimal_forms_with_a_clear_error() {
        for invalid_id in [
            "550e8400-e29b-41d4-a716-446655440000",
            "",
            " 42",
            "42 ",
            "+42",
            "-42",
            "42.0",
            "4.2e1",
            "9223372036854775808",
        ] {
            let error =
                serde_json::from_value::<NoteIdParams>(json!({ "id": invalid_id })).unwrap_err();
            assert!(
                error.to_string().contains("invalid note ID"),
                "{invalid_id:?} produced {error}"
            );
        }
    }
}
