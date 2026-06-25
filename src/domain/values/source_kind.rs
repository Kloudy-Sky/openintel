use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Reddit,
    X,
    Bluesky,
}

impl SourceKind {
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
}
