use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub type JsonValue = serde_json::Value;
pub type JsonObject = serde_json::Map<String, JsonValue>;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OkResponse {
    pub ok: bool,
}

pub fn lenient_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Option::<JsonValue>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}

pub fn lenient_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Option::<Vec<JsonValue>>::deserialize(deserializer)?;
    Ok(value
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect())
}

pub mod double_option {
    use super::*;

    pub fn serialize<S, T>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

pub fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    struct Patch {
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "double_option"
        )]
        title: Option<Option<String>>,
    }

    #[test]
    fn double_option_distinguishes_absent_null_and_value() {
        let absent = Patch { title: None };
        let cleared = Patch { title: Some(None) };
        let set = Patch {
            title: Some(Some("x".into())),
        };
        assert_eq!(serde_json::to_string(&absent).unwrap(), "{}");
        assert_eq!(
            serde_json::to_string(&cleared).unwrap(),
            r#"{"title":null}"#
        );
        assert_eq!(serde_json::to_string(&set).unwrap(), r#"{"title":"x"}"#);
        assert_eq!(serde_json::from_str::<Patch>("{}").unwrap(), absent);
        assert_eq!(
            serde_json::from_str::<Patch>(r#"{"title":null}"#).unwrap(),
            cleared
        );
    }
}
