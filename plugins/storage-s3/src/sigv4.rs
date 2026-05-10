//! Minimal AWS Signature Version 4 signing for bucket management.
//!
//! Only covers the subset needed for S3 `CreateBucket` and `ListBuckets`.
//! Full SigV4 for object-level operations is handled internally by OpenDAL.

use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn hmac_sha256_bytes(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn signing_key(secret_key: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date    = hmac_sha256_bytes(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    let k_region  = hmac_sha256_bytes(&k_date, region.as_bytes());
    let k_service = hmac_sha256_bytes(&k_region, service.as_bytes());
    hmac_sha256_bytes(&k_service, b"aws4_request")
}

/// Output of signing an AWS request.
pub struct Signed {
    pub authorization: String,
    pub x_amz_date: String,
    pub x_amz_content_sha256: String,
}

/// Sign an S3 request with AWS SigV4.
///
/// - `method`       — HTTP verb (`"GET"`, `"PUT"`, …)
/// - `uri`          — URL path only, e.g. `/my-bucket` or `/`
/// - `query_string` — pre-sorted query string, e.g. `""` or `"list-type=2"`
/// - `host`         — hostname only, e.g. `"abc.r2.cloudflarestorage.com"`
/// - `payload`      — request body bytes (empty slice for GET / bucket-level PUT)
pub fn sign(
    method: &str,
    uri: &str,
    query_string: &str,
    host: &str,
    payload: &[u8],
    access_key: &str,
    secret_key: &str,
    region: &str,
) -> Signed {
    let now = Utc::now();
    let date = now.format("%Y%m%d").to_string();
    let datetime = now.format("%Y%m%dT%H%M%SZ").to_string();

    let payload_hash = sha256_hex(payload);
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_headers = format!(
        "host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{datetime}\n"
    );
    let canonical_request = format!(
        "{method}\n{uri}\n{query_string}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );

    let cr_hash = sha256_hex(canonical_request.as_bytes());
    let credential_scope = format!("{date}/{region}/s3/aws4_request");
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{datetime}\n{credential_scope}\n{cr_hash}");

    let key = signing_key(secret_key, &date, region, "s3");
    let signature = hex::encode(hmac_sha256_bytes(&key, string_to_sign.as_bytes()));

    Signed {
        authorization: format!(
            "AWS4-HMAC-SHA256 Credential={access_key}/{credential_scope},\
             SignedHeaders={signed_headers},Signature={signature}"
        ),
        x_amz_date: datetime,
        x_amz_content_sha256: payload_hash,
    }
}
