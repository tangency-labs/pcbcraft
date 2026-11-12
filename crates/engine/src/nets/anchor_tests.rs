use super::*;

/// Fixed RFC-order UUIDs make every field boundary independently visible in codec vectors.
fn anchor(kind: AnchorKind, scope: ScopeKind) -> Anchor {
    let project = Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap();
    Anchor {
        kind,
        project,
        scope: Scope { kind: scope, id: if scope == ScopeKind::Board { Uuid::from_u128(1) } else { project } },
        occurrence: if kind == AnchorKind::Terminal { Uuid::nil() } else { Uuid::from_u128(2) },
        object: 9,
        terminal: if kind == AnchorKind::Terminal { "EP".into() } else { String::new() },
    }
}

/// Test every kind/scope tag, exact UUID bytes, big-endian numbers, lengths and opaque tokens.
#[test]
fn canonical_vectors_cover_every_kind_and_scope() {
    for (kind_tag, kind) in [AnchorKind::Label, AnchorKind::Port, AnchorKind::Terminal, AnchorKind::Junction].into_iter().enumerate() {
        for (scope_tag, scope) in [ScopeKind::Board, ScopeKind::System, ScopeKind::Schematic].into_iter().enumerate() {
            let anchor = anchor(kind, scope);
            let key = anchor.canonical_key().unwrap();
            assert_eq!(key[0], kind_tag as u8 + 1);
            assert_eq!(&key[1..17], &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
            assert_eq!(key[17], scope_tag as u8 + 1);
            assert_eq!(&key[18..34], anchor.scope.id.as_bytes());
            assert_eq!(&key[34..50], anchor.occurrence.as_bytes());
            assert_eq!(&key[50..58], &9_u64.to_be_bytes());
            assert_eq!(&key[58..62], &(anchor.terminal.len() as u32).to_be_bytes());
            assert_eq!(&key[62..], anchor.terminal.as_bytes());
        }
    }
}

/// Native sorting must match the encoded bytes, including 9/10 and length before token bytes.
#[test]
fn anchor_order_is_binary_and_not_rendered_or_lexical_token_order() {
    let a = anchor(AnchorKind::Terminal, ScopeKind::Schematic);
    let mut b = a.clone();
    b.object = 10;
    assert!(a < b);
    assert!(a.canonical_key().unwrap() < b.canonical_key().unwrap());
    let mut early = a.clone();
    early.project = Uuid::from_u128(1);
    early.scope.id = early.project;
    assert!(early < a);
    let mut short = a.clone();
    short.terminal = "Z".into();
    let mut long = a.clone();
    long.terminal = "AA".into();
    assert!(short < long);
    assert!(short.canonical_key().unwrap() < long.canonical_key().unwrap());
    let mut opaque = a.clone();
    opaque.terminal = "Ω".into();
    assert_eq!(&opaque.canonical_key().unwrap()[58..], &[0, 0, 0, 2, 0xce, 0xa9]);
    for invalid in [
        Anchor { occurrence: Uuid::from_u128(3), ..a.clone() },
        Anchor { terminal: String::new(), ..a.clone() },
        Anchor { object: 0, ..a.clone() },
        Anchor { project: Uuid::nil(), ..a.clone() },
        Anchor { scope: Scope { kind: ScopeKind::System, id: Uuid::from_u128(4) }, ..a.clone() },
    ] {
        assert!(invalid.canonical_key().is_err());
    }
}
