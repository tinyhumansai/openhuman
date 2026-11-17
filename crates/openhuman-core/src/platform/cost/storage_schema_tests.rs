use super::*;

#[test]
fn cost_collection_declares_its_time_index() {
    let specs = collection_specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, RECORDS);
    assert_eq!(specs[0].indexes[0].name, "by_time");
    specs[0].validate().unwrap();
}

#[test]
fn a_record_round_trips_through_its_body() {
    let usage = super::super::types::TokenUsage::new("m", 10, 5, 1.0, 2.0);
    let record = CostRecord::new("s", usage);
    let body = to_body(&record).unwrap();
    assert!(body["usage"].get("timestamp").is_none());
    let back = from_body(body).unwrap();
    assert_eq!(back.id, record.id);
    assert_eq!(
        back.usage.timestamp.timestamp_millis(),
        record.usage.timestamp.timestamp_millis()
    );
}
