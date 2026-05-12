use super::{sha256_hex, sign};

/// SHA-256 of the empty string, from NIST FIPS 180-4.
const SHA256_EMPTY: &str =
    "e3b0c44298fc1c149afbf4c8996fb924\
     27ae41e4649b934ca495991b7852b855";

/// SHA-256("hello"), from NIST test vectors.
const SHA256_HELLO: &str =
    "2cf24dba5fb0a30e26e83b2ac5b9e29e\
     1b161e5c1fa7425e73043362938b9824";

#[test]
fn sha256_hex_empty_input() {
    assert_eq!(sha256_hex(b""), SHA256_EMPTY);
}

#[test]
fn sha256_hex_known_value() {
    assert_eq!(sha256_hex(b"hello"), SHA256_HELLO);
}

#[test]
fn sha256_hex_output_is_lowercase_hex() {
    let result = sha256_hex(b"FreeBox");
    assert_eq!(result.len(), 64, "SHA-256 hex must be 64 chars");
    assert!(result.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
}

#[test]
fn signed_authorization_header_has_correct_prefix() {
    let signed = sign(
        "GET", "/", "", "example.s3.amazonaws.com",
        b"", "AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "us-east-1",
    );
    assert!(
        signed.authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"
        ),
        "authorization must embed the access key in the credential"
    );
    assert!(
        signed.authorization.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"),
        "signed headers list must be present"
    );
    assert!(
        signed.authorization.contains("Signature="),
        "signature field must be present"
    );
}

#[test]
fn signed_content_sha256_matches_payload() {
    let payload = b"Hello, S3!";
    let signed = sign(
        "PUT", "/my-bucket/my-key", "",
        "my-bucket.s3.amazonaws.com",
        payload,
        "ACCESS", "SECRET", "eu-west-1",
    );
    assert_eq!(signed.x_amz_content_sha256, sha256_hex(payload));
}

#[test]
fn sign_empty_payload_uses_empty_sha256() {
    let signed = sign(
        "GET", "/", "", "example.s3.amazonaws.com",
        b"", "ACCESS", "SECRET", "us-east-1",
    );
    assert_eq!(signed.x_amz_content_sha256, SHA256_EMPTY);
}

#[test]
fn signed_amz_date_is_iso8601_basic_format() {
    let signed = sign(
        "GET", "/", "", "example.s3.amazonaws.com",
        b"", "ACCESS", "SECRET", "us-east-1",
    );
    // Format: YYYYMMDDTHHmmSSZ  (16 chars, ends with 'Z')
    assert_eq!(signed.x_amz_date.len(), 16);
    assert!(signed.x_amz_date.ends_with('Z'));
    assert!(signed.x_amz_date.chars().nth(8) == Some('T'));
}

#[test]
fn different_credentials_produce_different_signatures() {
    let make = |access: &str, secret: &str| {
        sign("GET", "/", "", "s3.amazonaws.com", b"", access, secret, "us-east-1")
            .authorization
    };
    assert_ne!(
        make("KEY_A", "SECRET_A"),
        make("KEY_B", "SECRET_B"),
        "distinct credentials must yield distinct authorization headers"
    );
}

#[test]
fn different_regions_produce_different_signatures() {
    let make = |region: &str| {
        sign("GET", "/", "", "s3.amazonaws.com", b"", "ACCESS", "SECRET", region)
            .authorization
    };
    assert_ne!(
        make("us-east-1"),
        make("eu-west-1"),
        "distinct regions must yield distinct signatures"
    );
}
