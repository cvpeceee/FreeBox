use base64::{engine::general_purpose::STANDARD, Engine};

use super::{decode_name, format_size};

#[test]
fn format_size_bytes() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(1), "1 B");
    assert_eq!(format_size(1023), "1023 B");
}

#[test]
fn format_size_kilobytes() {
    assert_eq!(format_size(1024), "1.0 KB");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(1024 * 1024 - 1), "1024.0 KB");
}

#[test]
fn format_size_megabytes() {
    assert_eq!(format_size(1024 * 1024), "1.0 MB");
    assert_eq!(format_size(1024 * 1024 * 512), "512.0 MB");
}

#[test]
fn format_size_gigabytes() {
    assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    assert_eq!(format_size(1024 * 1024 * 1024 * 2), "2.0 GB");
}

#[test]
fn decode_name_valid_base64_utf8() {
    let name = "hello, world!";
    let encoded = STANDARD.encode(name.as_bytes());
    assert_eq!(decode_name(&encoded), name);
}

#[test]
fn decode_name_falls_back_to_raw_on_invalid_base64() {
    let raw = "not-base64!@#$";
    assert_eq!(decode_name(raw), raw);
}

#[test]
fn decode_name_falls_back_on_non_utf8_bytes() {
    // Valid base64, but the decoded bytes are not valid UTF-8.
    let invalid_utf8 = STANDARD.encode(&[0xFF_u8, 0xFE]);
    let result = decode_name(&invalid_utf8);
    // Falls back to returning the original base64 string.
    assert_eq!(result, invalid_utf8);
}

#[test]
fn decode_name_empty_string() {
    // STANDARD.encode("") == ""
    assert_eq!(decode_name(""), "");
}
