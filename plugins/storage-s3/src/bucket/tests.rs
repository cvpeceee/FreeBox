use super::{extract_tag, parse_list_buckets_xml};

#[test]
fn parse_single_bucket_with_creation_date() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListAllMyBucketsResult>
  <Buckets>
    <Bucket>
      <Name>my-photos</Name>
      <CreationDate>2023-06-01T09:00:00.000Z</CreationDate>
    </Bucket>
  </Buckets>
</ListAllMyBucketsResult>"#;

    let buckets = parse_list_buckets_xml(xml);
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].name, "my-photos");
    assert_eq!(
        buckets[0].creation_date.as_deref(),
        Some("2023-06-01T09:00:00.000Z")
    );
}

#[test]
fn parse_multiple_buckets() {
    let xml = r#"<ListAllMyBucketsResult><Buckets>
  <Bucket><Name>alpha</Name><CreationDate>2023-01-01T00:00:00.000Z</CreationDate></Bucket>
  <Bucket><Name>beta</Name><CreationDate>2023-02-01T00:00:00.000Z</CreationDate></Bucket>
  <Bucket><Name>gamma</Name></Bucket>
</Buckets></ListAllMyBucketsResult>"#;

    let buckets = parse_list_buckets_xml(xml);
    assert_eq!(buckets.len(), 3);
    assert_eq!(buckets[0].name, "alpha");
    assert_eq!(buckets[1].name, "beta");
    assert_eq!(buckets[2].name, "gamma");
    assert!(buckets[2].creation_date.is_none());
}

#[test]
fn parse_empty_bucket_list() {
    let xml = "<ListAllMyBucketsResult><Buckets></Buckets></ListAllMyBucketsResult>";
    assert!(parse_list_buckets_xml(xml).is_empty());
}

#[test]
fn parse_skips_buckets_with_empty_name() {
    let xml = r#"<Buckets>
  <Bucket><Name></Name><CreationDate>2023-01-01T00:00:00.000Z</CreationDate></Bucket>
  <Bucket><Name>valid</Name></Bucket>
</Buckets>"#;
    let buckets = parse_list_buckets_xml(xml);
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].name, "valid");
}

#[test]
fn parse_non_xml_returns_empty() {
    assert!(parse_list_buckets_xml("not xml at all").is_empty());
    assert!(parse_list_buckets_xml("").is_empty());
}

#[test]
fn extract_tag_finds_value() {
    let s = "<Root><Name>hello</Name></Root>";
    assert_eq!(extract_tag(s, "Name"), Some("hello"));
}

#[test]
fn extract_tag_returns_none_for_missing_tag() {
    let s = "<Root></Root>";
    assert_eq!(extract_tag(s, "Name"), None);
}

#[test]
fn bucket_info_name_field_is_stored() {
    let xml = "<Buckets><Bucket><Name>freebox-data</Name></Bucket></Buckets>";
    let buckets = parse_list_buckets_xml(xml);
    assert_eq!(buckets[0].name, "freebox-data");
}
