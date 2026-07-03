use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Reddit,
    X,
    Bluesky,
}

impl SourceKind {
    /// The full set of social sources, in canonical order — the single source of
    /// truth for the request defaults (`AppConfig` and the MCP `request_from`).
    pub const ALL: [SourceKind; 3] = [SourceKind::Reddit, SourceKind::X, SourceKind::Bluesky];

    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Reddit => "reddit",
            SourceKind::X => "x",
            SourceKind::Bluesky => "bluesky",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_matches() {
        assert_eq!(SourceKind::Reddit.as_str(), "reddit");
        assert_eq!(SourceKind::X.as_str(), "x");
        assert_eq!(SourceKind::Bluesky.as_str(), "bluesky");
    }

    #[test]
    fn serializes_lowercase() {
        let json = serde_json::to_string(&SourceKind::Bluesky).unwrap();
        assert_eq!(json, "\"bluesky\"");
        assert_eq!(serde_json::to_string(&SourceKind::X).unwrap(), "\"x\"");
        assert_eq!(
            serde_json::to_string(&SourceKind::Reddit).unwrap(),
            "\"reddit\""
        );
    }

    #[test]
    fn all_lists_every_variant_in_order() {
        assert_eq!(
            SourceKind::ALL,
            [SourceKind::Reddit, SourceKind::X, SourceKind::Bluesky]
        );
    }
}
