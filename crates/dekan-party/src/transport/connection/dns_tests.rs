use super::*;
use serde_json::json;

fn reply(answers: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"Status": 0, "Answer": answers})).expect("json")
}

#[test]
fn only_matching_public_a_records_are_used_and_deduplicated() {
    let bytes = reply(json!([
        {"name": "RELAY.EXAMPLE.", "type": 1, "data": "104.21.40.9"},
        {"name": "relay.example", "type": 1, "data": "104.21.40.9"},
        {"name": "other.example", "type": 1, "data": "172.67.173.214"},
        {"name": "relay.example", "type": 28, "data": "2606:4700::1"},
        {"name": "relay.example", "type": 1, "data": "invalid"},
        {"name": "relay.example", "type": 1, "data": "127.0.0.1"}
    ]));
    assert_eq!(
        parse(&bytes, "relay.example").expect("valid"),
        vec!["104.21.40.9:443".parse::<SocketAddr>().expect("address")]
    );
}

#[test]
fn failure_truncation_unchecked_malformed_and_oversized_answers_are_rejected() {
    for bytes in [
        br#"{"Status":2}"#.to_vec(),
        br#"{"Status":0,"TC":true}"#.to_vec(),
        br#"{"Status":0,"CD":true}"#.to_vec(),
        br#"{"Answer":[]}"#.to_vec(),
        b"invalid".to_vec(),
        vec![b' '; MAX_BODY + 1],
    ] {
        assert!(parse(&bytes, "relay.example").is_err());
    }
    assert!(
        parse(br#"{"Status":0}"#, "relay.example")
            .expect("empty answer")
            .is_empty()
    );
}

#[test]
fn address_count_is_bounded_and_private_or_special_ranges_are_rejected() {
    for ip in [
        "0.1.2.3",
        "10.0.0.1",
        "127.0.0.1",
        "169.254.1.1",
        "172.16.0.1",
        "192.168.0.1",
        "192.0.0.1",
        "192.0.2.1",
        "198.51.100.1",
        "203.0.113.1",
        "100.64.0.1",
        "100.127.255.254",
        "198.18.0.1",
        "224.0.0.1",
        "240.0.0.1",
        "255.255.255.255",
    ] {
        assert!(!public_ipv4(ip.parse().expect("ip")), "{ip}");
    }
    let answers: Vec<_> = (1..=20)
        .map(|n| json!({"name":"relay.example", "type":1, "data":format!("104.21.40.{n}")}))
        .collect();
    assert_eq!(
        parse(&reply(json!(answers)), "relay.example")
            .expect("valid")
            .len(),
        MAX_ADDRESSES
    );
}
