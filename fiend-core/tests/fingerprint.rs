use fiend_core::fingerprint::parse_client_hello;
use fiend_core::fingerprint::{
    DEFAULT_RECORD_MAX, FieldOffset, GreaseKind, KeyShareSpec, P256, P256_SIZE, Template, X25519,
    X25519_MLKEM768, X25519_MLKEM768_SIZE, X25519_SIZE,
};

const TEMPLATE_JSON: &str = include_str!("../testdata/template.json");

fn sample() -> Template {
    Template::from_json(TEMPLATE_JSON).expect("load sample template")
}

#[test]
fn sample_json_loads_and_verifies() {
    let template = sample();
    assert_eq!(template.domain, "www.wikipedia.org");
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
    assert_eq!(template.grease.len(), 7);
    assert_eq!(template.raw.len(), 1925);
    let groups: Vec<u16> = template.key_shares.iter().map(|spec| spec.group).collect();
    assert_eq!(groups, vec![X25519_MLKEM768, X25519]);
    assert!(template.cipher_suites.contains(&0x1301));
    assert!(template.cipher_suites.contains(&0x1302));
    assert!(template.cipher_suites.contains(&0x1303));
    assert!(!template.early_data);
    let ech = template.ech.expect("ECH GREASE extension");
    assert_eq!(ech.enc.size, 32);
    assert_eq!(ech.payload.size, 144);
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

fn dynamic_ranges(template: &Template) -> Vec<std::ops::Range<usize>> {
    let mut ranges = vec![3..5, 6..9, 11..43, 44..76];
    let ech = template.ech.expect("ECH");
    ranges.push(template.extensions_offset..template.extensions_offset + 2);
    ranges.push(ech.extension + 2..ech.extension + 4);
    for spec in &template.key_shares {
        ranges.push(spec.offset..spec.offset + spec.size);
    }
    for field in &template.grease {
        ranges.push(field.value_offset..field.value_offset + 2);
    }
    ranges.push(ech.config_id..ech.config_id + 1);
    ranges.push(ech.enc.offset..ech.enc.offset + ech.enc.size);
    ranges.push(ech.payload.offset - 2..template.raw.len());
    ranges
}

#[test]
fn build_regenerates_dynamic_fields_and_keeps_static_bytes() {
    let template = sample();
    let ranges = dynamic_ranges(&template);
    let built = template
        .build_in_capture_order(&[7u8; 32])
        .expect("build ClientHello");
    let reparsed = parse_client_hello(&built.record).expect("built ClientHello parses");
    assert_eq!(reparsed.domain, template.domain);

    for (index, (&before, &after)) in template.raw.iter().zip(&built.record).enumerate() {
        if !ranges.iter().any(|range| range.contains(&index)) {
            assert_eq!(before, after, "static byte {index} changed");
        }
    }
    let ech = template.ech.expect("ECH");
    let tail_start = ech.payload.offset + ech.payload.size;
    let shift = built.record.len() - template.raw.len();
    for index in tail_start..template.raw.len() {
        let grease = template
            .grease
            .iter()
            .any(|field| (field.value_offset..field.value_offset + 2).contains(&index));
        if !grease {
            assert_eq!(
                template.raw[index],
                built.record[index + shift],
                "tail byte {index}"
            );
        }
    }
    assert_ne!(&template.raw[11..43], &built.record[11..43]);
    let share = &template.key_shares[0];
    assert_ne!(
        &template.raw[share.offset..share.offset + share.size],
        &built.record[share.offset..share.offset + share.size]
    );
}

fn grease_value(template: &Template, record: &[u8], offset: usize) -> u16 {
    let ech = template.ech.expect("ECH");
    let offset = if offset > ech.payload.offset {
        offset + record.len() - template.raw.len()
    } else {
        offset
    };
    u16::from_be_bytes([record[offset], record[offset + 1]])
}

#[test]
fn grease_values_follow_slot_rules() {
    let template = sample();
    let slot = |kind: GreaseKind| -> Vec<usize> {
        template
            .grease
            .iter()
            .filter(|field| field.kind == kind)
            .map(|field| field.value_offset)
            .collect()
    };
    let groups = slot(GreaseKind::Group);
    let first = slot(GreaseKind::Extension1)[0];
    let second = slot(GreaseKind::Extension2)[0];
    let cipher = slot(GreaseKind::Cipher)[0];
    assert_eq!(groups.len(), 2);

    let mut cipher_values = std::collections::HashSet::new();
    let mut cipher_matches_group = 0;
    for _ in 0..128 {
        let built = template
            .build_in_capture_order(&[1u8; 32])
            .expect("build with GREASE");
        for field in &template.grease {
            let value = grease_value(&template, &built.record, field.value_offset);
            assert_eq!(value & 0x0f0f, 0x0a0a);
            assert_eq!(value >> 8, value & 0xff);
        }
        assert_ne!(
            grease_value(&template, &built.record, first),
            grease_value(&template, &built.record, second)
        );
        assert_eq!(
            grease_value(&template, &built.record, groups[0]),
            grease_value(&template, &built.record, groups[1])
        );
        let cipher_value = grease_value(&template, &built.record, cipher);
        cipher_values.insert(cipher_value);
        if cipher_value == grease_value(&template, &built.record, groups[0]) {
            cipher_matches_group += 1;
        }
    }
    assert!(cipher_values.len() > 4);
    assert!(cipher_matches_group < 64);
}

#[test]
fn grease_data_stays_static() {
    let template = sample();
    for _ in 0..16 {
        let built = template
            .build_in_capture_order(&[5u8; 32])
            .expect("build GREASE data");
        for field in &template.grease {
            if let (Some(offset), Some(size)) = (field.data_offset, field.data_size) {
                let shift = if offset > template.ech.expect("ECH").payload.offset {
                    built.record.len() - template.raw.len()
                } else {
                    0
                };
                assert_eq!(
                    &built.record[offset + shift..offset + shift + size],
                    &template.raw[offset..offset + size]
                );
            }
        }
    }
}

#[test]
fn ech_grease_is_fresh_and_varies_in_size() {
    let template = sample();
    let ech = template.ech.expect("ECH");
    let mut sizes = std::collections::HashSet::new();
    let mut encs = std::collections::HashSet::new();
    for _ in 0..64 {
        let built = template
            .build_in_capture_order(&[6u8; 32])
            .expect("build ECH");
        let parsed = parse_client_hello(&built.record).expect("parse built ClientHello");
        let built_ech = parsed.ech.expect("ECH survives build");
        assert_eq!(built_ech.config_id, ech.config_id);
        assert!([144, 176, 208, 240].contains(&built_ech.payload.size));
        assert_eq!(
            built.record.len() as isize - template.raw.len() as isize,
            built_ech.payload.size as isize - ech.payload.size as isize
        );
        let enc = &built.record[ech.enc.offset..ech.enc.offset + 32];
        assert_eq!(enc[31] & 0x80, 0);
        encs.insert(enc.to_vec());
        sizes.insert(built_ech.payload.size);
    }
    assert_eq!(encs.len(), 64);
    assert_eq!(sizes.len(), 4);
}

fn extension_kinds(template: &Template) -> Vec<u16> {
    template
        .extension_blocks()
        .iter()
        .map(|(kind, _)| *kind)
        .collect()
}

#[test]
fn chrome_templates_permute_extensions_like_chrome() {
    let template = sample();
    assert!(template.permutes_extensions());
    let original = extension_kinds(&template);
    let grease = |kind: &u16| kind & 0x0f0f == 0x0a0a;
    let mut orders = std::collections::HashSet::new();
    for _ in 0..32 {
        let built = template.build(&[8u8; 32]).expect("build permuted");
        let parsed = parse_client_hello(&built.record).expect("permuted ClientHello parses");
        let kinds = extension_kinds(&parsed);
        assert_eq!(kinds.len(), original.len());
        for (index, kind) in original.iter().enumerate() {
            if grease(kind) || *kind == 0x0029 || *kind == 0x0015 {
                assert!(
                    grease(&kinds[index]) == grease(kind),
                    "slot {index} stays in place"
                );
            }
        }
        let mut sorted: Vec<u16> = kinds.iter().copied().filter(|kind| !grease(kind)).collect();
        let mut expected: Vec<u16> = original
            .iter()
            .copied()
            .filter(|kind| !grease(kind))
            .collect();
        sorted.sort();
        expected.sort();
        assert_eq!(sorted, expected);
        for (spec, share) in parsed.key_shares.iter().zip(&built.key_shares.shares) {
            assert_eq!(
                &built.record[spec.offset..spec.offset + spec.size],
                share.public.as_slice()
            );
        }
        parsed
            .verify(&built.record)
            .expect("permuted ClientHello is consistent");
        orders.insert(
            kinds
                .into_iter()
                .filter(|kind| !grease(kind))
                .collect::<Vec<_>>(),
        );
    }
    assert!(orders.len() > 16, "the order changes between handshakes");
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
    let json = TEMPLATE_JSON.replacen("\"record_max\": 16384", "\"record_max\": 0", 1);
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
        template.grease.clear();
        template.ech = None;
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
    template.grease.clear();
    template.ech = None;
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
    let second = template.build_at(&secret, 4_501).expect("build PSK fields");
    assert_ne!(&built.record[0..40], &template.raw[0..40]);
    assert_ne!(&built.record[0..40], &second.record[0..40]);
    assert_ne!(&built.record[40..44], &template.raw[40..44]);
    assert_ne!(&built.record[40..44], &second.record[40..44]);
    assert_ne!(&built.record[44..76], &[0u8; 32]);
    assert_ne!(&built.record[44..76], &second.record[44..76]);
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
