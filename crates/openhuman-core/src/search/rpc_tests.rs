use super::*;

#[test]
fn optional_string_array_trims_and_drops_blank_entries() {
    let params = Map::from_iter([("categories".to_string(), json!([" web ", "", "  ", "news"]))]);

    let values = optional_string_array(&params, "categories").expect("string array");

    assert_eq!(values, vec!["web", "news"]);
}
