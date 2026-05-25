//! Derived graph edge shape.

use serde::{Deserialize, Serialize};

/// A derived co-occurrence edge between two entities.
///
/// Not a triple in the classical sense — there's no explicit predicate. The
/// `weight` field is the count of distinct nodes the pair has both appeared
/// on, which serves as a cheap proxy for relationship strength.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub subject: String,
    pub object: String,
    pub weight: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn graph_edge_roundtrips_via_serde() {
        let edge = GraphEdge {
            subject: "person:alice".into(),
            object: "project:openhuman".into(),
            weight: 3,
        };
        let value = serde_json::to_value(&edge).unwrap();
        assert_eq!(
            value,
            json!({
                "subject": "person:alice",
                "object": "project:openhuman",
                "weight": 3
            })
        );

        let decoded: GraphEdge = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, edge);
    }
}
