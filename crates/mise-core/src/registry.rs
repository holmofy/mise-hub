use crate::error::CoreError;
use crate::types::ToolEntry;

pub fn parse_registry(json: &str) -> Result<Vec<ToolEntry>, CoreError> {
    Ok(serde_json::from_str(json)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entries_and_defaults_missing_fields() {
        let json = r#"[
          {"short":"node","backends":["core:node"],"bins":["node","npm"],"description":"JavaScript runtime","aliases":["nodejs"]},
          {"short":"bare"}
        ]"#;
        let entries = parse_registry(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].description.as_deref(),
            Some("JavaScript runtime")
        );
        assert!(entries[1].description.is_none());
        assert!(entries[1].aliases.is_empty());
        assert!(entries[1].bins.is_empty());
    }

    #[test]
    fn invalid_json_is_an_error_not_a_panic() {
        assert!(parse_registry("not json").is_err());
    }
}
