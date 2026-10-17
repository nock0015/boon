use pbdems2::entity::{
    BareCharEncoding, DecodeProfile, FlattenedField, FlattenedSerializer,
    FlattenedSerializerDefinition, PreciseQAngleMode, SerializerContainer,
};

const STRING_PROFILE: DecodeProfile = DecodeProfile::new(
    BareCharEncoding::NullTerminatedString,
    PreciseQAngleMode::Centered,
)
.with_ammo_field("m_ammo");

const SCALAR_PROFILE: DecodeProfile =
    DecodeProfile::new(BareCharEncoding::UnsignedVarint, PreciseQAngleMode::Raw)
        .with_pitch_yaw_qangles();

fn decoder_name(
    profile: DecodeProfile,
    var_type: &str,
    var_name: &str,
    bit_count: Option<i32>,
    encoder: Option<&str>,
) -> String {
    let mut symbols = vec!["CTest".to_owned(), var_type.to_owned(), var_name.to_owned()];
    let var_encoder_sym = encoder.map(|encoder| {
        symbols.push(encoder.to_owned());
        3
    });

    let serializers = SerializerContainer::parse(
        FlattenedSerializer::new(
            vec![FlattenedSerializerDefinition::new(Some(0), vec![0])],
            symbols,
            vec![
                FlattenedField::new(Some(1), Some(2))
                    .with_bit_count(bit_count)
                    .with_encoder_sym(var_encoder_sym),
            ],
        ),
        profile,
    )
    .unwrap();

    format!(
        "{:?}",
        serializers.get("CTest").unwrap().fields[0].metadata.decoder
    )
}

#[test]
fn bare_char_follows_the_profile() {
    assert_eq!(
        decoder_name(STRING_PROFILE, "char", "m_value", None, None),
        "String"
    );
    assert_eq!(
        decoder_name(SCALAR_PROFILE, "char", "m_value", None, None),
        "U64"
    );
}

#[test]
fn ammo_field_is_an_explicit_override() {
    assert_eq!(
        decoder_name(STRING_PROFILE, "uint16", "m_ammo", None, None),
        "Ammo"
    );
    assert_eq!(
        decoder_name(SCALAR_PROFILE, "uint16", "m_ammo", None, None),
        "U64"
    );
}

#[test]
fn angle_conventions_follow_the_profile() {
    assert_eq!(
        decoder_name(
            SCALAR_PROFILE,
            "QAngle",
            "m_angle",
            Some(10),
            Some("qangle_pitch_yaw"),
        ),
        "QAnglePitchYaw { bit_count: 10 }"
    );
    assert_eq!(
        decoder_name(
            SCALAR_PROFILE,
            "QAngle",
            "m_angle",
            Some(20),
            Some("qangle_precise"),
        ),
        "QAnglePreciseRaw"
    );
    assert_eq!(
        decoder_name(
            STRING_PROFILE,
            "QAngle",
            "m_angle",
            Some(20),
            Some("qangle_precise"),
        ),
        "QAnglePrecise"
    );
}

#[test]
fn game_defined_type_overrides_are_data_driven() {
    const PROFILE: DecodeProfile = DecodeProfile::new(
        BareCharEncoding::NullTerminatedString,
        PreciseQAngleMode::Centered,
    )
    .with_symbolic_array_lengths(&[("GAME_COUNT", 7)])
    .with_pointer_types(&["CGamePointer"])
    .with_dynamic_serializer_types(&["CGameDynamicArray"]);

    let serializers = SerializerContainer::parse(
        FlattenedSerializer::new(
            vec![FlattenedSerializerDefinition::new(Some(0), vec![0, 1, 2])],
            vec![
                "CTest".into(),
                "uint32[GAME_COUNT]".into(),
                "m_fixed".into(),
                "CGamePointer".into(),
                "m_pointer".into(),
                "CGameDynamicArray".into(),
                "m_dynamic".into(),
            ],
            vec![
                FlattenedField::new(Some(1), Some(2)),
                FlattenedField::new(Some(3), Some(4)),
                FlattenedField::new(Some(5), Some(6)),
            ],
        ),
        PROFILE,
    )
    .unwrap();

    let fields = &serializers.get("CTest").unwrap().fields;
    assert_eq!(fields[0].metadata.fixed_array_length(), Some(7));
    assert!(fields[1].metadata.is_pointer());
    assert!(fields[2].metadata.is_dynamic_serializer_array());
}

#[test]
fn fixed8_requires_an_explicit_encoder_in_both_profiles() {
    for profile in [STRING_PROFILE, SCALAR_PROFILE] {
        for (var_type, default, fixed) in [
            ("int8", "I64", "I64Fixed8"),
            ("uint8", "U64", "U64Fixed8"),
            ("MoveType_t", "U64", "U64Fixed8"),
        ] {
            assert_eq!(
                decoder_name(profile, var_type, "m_value", None, None),
                default
            );
            assert_eq!(
                decoder_name(profile, var_type, "m_value", None, Some("fixed8")),
                fixed
            );
        }
    }
}
