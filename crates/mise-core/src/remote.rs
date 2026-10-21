use crate::error::CoreError;
use crate::types::RemoteVersion;

pub fn parse_remote_versions(json: &str) -> Result<Vec<RemoteVersion>, CoreError> {
    let mut versions: Vec<RemoteVersion> = serde_json::from_str(json)?;
    versions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sorts_newest_first() {
        let json = r#"[
          {"version":"20.10.0","created_at":"2023-01-01T00:00:00.0Z"},
          {"version":"20.11.0","created_at":"2024-01-01T00:00:00.0Z"},
          {"version":"20.9.0","created_at":"2022-01-01T00:00:00.0Z"}
        ]"#;
        let versions = parse_remote_versions(json).unwrap();
        assert_eq!(versions[0].version, "20.11.0");
        assert_eq!(versions[2].version, "20.9.0");
    }

    #[test]
    fn missing_created_at_does_not_panic() {
        let versions = parse_remote_versions(r#"[{"version":"1.0.0"}]"#).unwrap();
        assert_eq!(versions[0].version, "1.0.0");
    }
}
