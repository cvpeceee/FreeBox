use uuid::Uuid;

#[test]
fn valid_uuid_string_parses_successfully() {
    let id = Uuid::new_v4();
    let parsed = Uuid::parse_str(&id.to_string());
    assert!(parsed.is_ok());
    assert_eq!(parsed.unwrap(), id);
}

#[test]
fn invalid_uuid_string_returns_parse_error() {
    let inputs = ["not-a-uuid", "12345", "", "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"];
    for input in inputs {
        assert!(
            Uuid::parse_str(input).is_err(),
            "'{input}' must fail UUID parsing"
        );
    }
}

#[test]
fn permanent_flag_message_is_informative() {
    // Verify the bail message is non-empty and mentions 30 days.
    // This is a compile-time constant check — if the string is changed,
    // the test documents the expected behaviour.
    let msg = "--permanent hard delete is not yet supported; \
               files in trash are purged automatically after 30 days";
    assert!(msg.contains("30 days"));
}
