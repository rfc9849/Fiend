use fiend_core::crypto;
use fiend_core::fingerprint::parse_client_hello;
use fiend_core::fingerprint::{
    DEFAULT_RECORD_MAX, FieldOffset, GreaseField, KeyShareSpec, P256, P256_SIZE, Template, X25519,
    X25519_MLKEM768, X25519_MLKEM768_SIZE, X25519_SIZE,
};

const TEMPLATE_JSON: &str = include_str!("../testdata/template.json");

fn sample() -> Template {
    Template::from_json(TEMPLATE_JSON).expect("load sample template")
}

#[test]
fn sample_json_loads_and_verifies() {
    let template = sample();
    assert_eq!(template.domain, "cdn.example.co.uk");
    assert_eq!(template.record_max, DEFAULT_RECORD_MAX);
    assert_eq!(
        template.tls_random,
        FieldOffset {
            offset: 11,
            size: 32
        }
    );
    assert_eq!(
        template.session_id,
        FieldOffset {
            offset: 44,
            size: 32
        }
    );
    assert_eq!(
        template.key_share,
        FieldOffset {
            offset: 231,
            size: 32
        }
    );
    assert!(template.grease.is_empty());
    assert_eq!(template.raw.len(), 517);
    assert_eq!(template.key_shares.len(), 1);
    assert_eq!(template.key_shares[0].group, 0x001d);
    assert!(template.cipher_suites.contains(&0x1301));
    assert!(template.cipher_suites.contains(&0x1302));
    assert!(template.cipher_suites.contains(&0x1303));
    assert!(!template.early_data);
    template.verify(&template.raw).expect("verify sample bytes");
}

#[test]
fn parsed_sample_matches_json_metadata() {
    let template = sample();
    let parsed = parse_client_hello(&template.raw).expect("parse sample ClientHello");
    assert_eq!(parsed.domain, template.domain);
    assert_eq!(parsed.tls_random, template.tls_random);
    assert_eq!(parsed.session_id, template.session_id);
    assert_eq!(parsed.key_share, template.key_share);
    assert_eq!(parsed.key_shares, template.key_shares);
    assert!(!parsed.cipher_suites.is_empty());
    assert_eq!(parsed.grease, template.grease);
    parsed.verify(&template.raw).expect("verify parsed bytes");
}

#[test]
fn build_regenerates_dynamic_fields_and_keeps_static_bytes() {
    let template = sample();
    let built = template.build(&[7u8; 32]).expect("build ClientHello");
    assert_eq!(built.record.len(), template.raw.len());

    let dynamic = |index: usize| {
        let in_random = (11..43).contains(&index);
        let in_session = (44..76).contains(&index);
        let in_key_share = (231..263).contains(&index);
        in_random || in_session || in_key_share
    };
    for (index, (&before, &after)) in template.raw.iter().zip(&built.record).enumerate() {
        if !dynamic(index) {
            assert_eq!(before, after, "static byte {index} changed");
        }
    }
    assert_ne!(&template.raw[11..43], &built.record[11..43]);
    assert_ne!(&template.raw[231..263], &built.record[231..263]);
}

#[test]
fn grease_fields_use_one_connection_value() {
    let mut template = sample();
    template.grease = vec![
        GreaseField {
            value_offset: 78,
            data_offset: None,
            data_size: None,
        },
        GreaseField {
            value_offset: 80,
            data_offset: None,
            data_size: None,
        },
        GreaseField {
            value_offset: 82,
            data_offset: None,
            data_size: None,
        },
    ];
    let built = template.build(&[1u8; 32]).expect("build with GREASE");
    let first = built.record[78];
    assert_eq!(first & 0x0f, 0x0a);
    for field in &template.grease {
        assert_eq!(built.record[field.value_offset], first);
        assert_eq!(built.record[field.value_offset + 1], first);
    }
}

#[test]
fn record_max_is_validated() {
    let mut template = sample();
    template.record_max = 0;
    assert!(template.build(&[0u8; 32]).is_err());
    template.record_max = DEFAULT_RECORD_MAX + 1;
    assert!(template.build(&[0u8; 32]).is_err());
}

#[test]
fn explicit_zero_record_max_uses_default() {
    let json = TEMPLATE_JSON.replacen('{', "{\n  \"record_max\": 0,", 1);
    let template = Template::from_json(&json).expect("load template");
    assert_eq!(template.record_max, DEFAULT_RECORD_MAX);
}

#[test]
fn build_supports_all_key_share_groups() {
    for (group, size) in [
        (X25519, X25519_SIZE),
        (P256, P256_SIZE),
        (X25519_MLKEM768, X25519_MLKEM768_SIZE),
    ] {
        let mut template = sample();
        template.raw = vec![0u8; size + 64];
        template.key_shares = vec![KeyShareSpec {
            group,
            offset: 32,
            size,
            public: vec![0u8; size],
        }];
        template.key_share = FieldOffset { offset: 32, size };

        let built = template.build(&[3u8; 32]).expect("build key share");
        assert_eq!(built.key_shares.shares.len(), 1);
        assert_eq!(built.key_shares.shares[0].group, group);
        assert_eq!(
            &built.record[32..32 + size],
            built.key_shares.shares[0].public.as_slice()
        );
        assert_ne!(built.key_shares.shares[0].public, vec![0u8; size]);
        if group == P256 {
            assert_eq!(built.key_shares.shares[0].public[0], 0x04);
        }
    }
}

#[test]
fn build_regenerates_psk_fields() {
    let mut template = sample();
    template.raw = vec![0u8; 200];
    template.key_shares = vec![KeyShareSpec {
        group: X25519,
        offset: 100,
        size: X25519_SIZE,
        public: vec![0u8; X25519_SIZE],
    }];
    template.key_share = FieldOffset {
        offset: 100,
        size: X25519_SIZE,
    };
    template.psk_identity = Some(FieldOffset {
        offset: 0,
        size: 40,
    });
    template.ticket_age = Some(FieldOffset {
        offset: 40,
        size: 4,
    });
    template.psk_binder = Some(FieldOffset {
        offset: 44,
        size: 32,
    });

    let secret = [9u8; 32];
    let built = template.build_at(&secret, 4_500).expect("build PSK fields");
    assert_eq!(
        &built.record[0..32],
        crypto::derive_psk_identity(&secret, 4_500)
    );
    assert_eq!(&built.record[40..44], crypto::ticket_age(&secret, 4_500));
    assert_ne!(&built.record[44..76], &[0u8; 32]);
}

#[test]
fn build_regenerates_grease_data() {
    let mut template = sample();
    template.grease = vec![GreaseField {
        value_offset: 78,
        data_offset: Some(80),
        data_size: Some(8),
    }];
    let built = template.build(&[5u8; 32]).expect("build GREASE data");
    assert_eq!(built.record[78], built.record[79]);
    assert_eq!(built.record[78] & 0x0f, 0x0a);
    assert_ne!(&built.record[80..88], &[0u8; 8]);
}

#[test]
fn save_and_load_round_trip() {
    let template = sample();
    let directory =
        std::env::temp_dir().join(format!("fiend-core-template-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create temp directory");
    let path = directory.join("template.json");
    template.save(&path).expect("save template");
    let loaded = Template::load(directory.join("template")).expect("load template");
    assert_eq!(loaded.domain, template.domain);
    assert_eq!(loaded.raw, template.raw);
    assert_eq!(loaded.grease, template.grease);
    assert_eq!(loaded.key_shares, template.key_shares);
    std::fs::remove_dir_all(directory).expect("remove temp directory");
}
