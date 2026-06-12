use evtx::RecordId;

#[test]
fn record_id_is_public() {
    let id: RecordId = 42;
    assert_eq!(id, 42);
}
