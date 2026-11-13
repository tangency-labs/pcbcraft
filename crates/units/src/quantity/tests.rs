use super::*;

/// Input aliases normalize to one persisted identity; canonical constructors reject
/// display spellings and precision changes rather than silently reinterpreting them.
#[test]
fn registry_validation_keeps_aliases_out_of_canonical_records() {
    let resistance = definition_for_unit("Ohm").unwrap();
    assert_eq!(resistance.quantity, Quantity::Resistance);
    assert_eq!(resistance.display_symbol, "Ω");
    for alias in ["Ohm", "ohm", "ohms", "Ω", "R"] {
        assert!(resistance.accepts(alias));
        let parsed = Quantity::Resistance.parse(&format!("0.1{alias}")).unwrap();
        assert_eq!(parsed.value(), "0.1");
        assert_eq!(parsed.unit(), "Ohm");
        assert_eq!(parsed.display(), "100 mΩ");
    }
    for alias in ["R", "Ω", "mOhm", "ohm"] {
        assert!(definition_for_unit(alias).is_none());
        assert!(CanonicalQuantity::new("0.1", alias).is_err());
    }
    for value in ["0.10", "-0", "1e-3", "0.0000000000000000001", "NaN"] {
        assert!(CanonicalQuantity::new(value, "F").is_err(), "{value}");
    }
    assert!(CanonicalQuantity::new("1", "unknown").is_err());
    assert_eq!(Quantity::Capacitance.parse("1fF").unwrap().into_parts(), ("0.000000000000001".into(), "F".into()));
}

/// The Ohm sign (U+2126) and Kelvin sign (U+212A) are Unicode canonical equivalents of Ω and
/// K. Datasheet text and character pickers produce them, so they parse like the letters,
/// alone or after an SI prefix and with or without a space.
#[test]
fn ohm_and_kelvin_signs_parse_like_their_letters() {
    let expected = Quantity::Resistance.parse("4.7kΩ").unwrap().into_parts();
    for text in ["4.7k\u{2126}", "4.7 k\u{2126}", "4700\u{2126}"] {
        assert_eq!(Quantity::Resistance.parse(text).unwrap().into_parts(), expected, "{text}");
    }
    assert_eq!(Quantity::Resistance.parse("100m\u{2126}").unwrap().value(), "0.1");
    assert_eq!(Quantity::Temperature.parse("300\u{212a}").unwrap().into_parts(), ("300".into(), "K".into()));
}

/// Display text uses engineering SI prefixes, keeps every digit and parses back to the stored value.
#[test]
fn quantities_display_with_si_prefixes_and_round_trip() {
    use super::display_quantity;
    for (quantity, value, shown) in [
        (Quantity::Capacitance, "0.0000001", "100 nF"),
        (Quantity::Resistance, "4700", "4.7 kΩ"),
        (Quantity::Frequency, "32768", "32.768 kHz"),
        (Quantity::Resistance, "0", "0 Ω"),
        (Quantity::Voltage, "-0.0033", "-3.3 mV"),
        (Quantity::Voltage, "1", "1 V"),
        (Quantity::Voltage, "999.999", "999.999 V"),
        (Quantity::Resistance, "1000", "1 kΩ"),
        (Quantity::Resistance, "1000000", "1 MΩ"),
        (Quantity::Current, "0.000001", "1 µA"),
        (Quantity::Capacitance, "0.000000000001", "1 pF"),
        (Quantity::Capacitance, "0.000000000000001", "1 fF"),
        (Quantity::Capacitance, "0.000000000000000001", "1 aF"),
        (Quantity::Frequency, "1000000000000000", "1 PHz"),
        (Quantity::Frequency, "1000000000000000000", "1 EHz"),
        (Quantity::Frequency, "1000000000000000000000", "1000 EHz"),
        (Quantity::Inductance, "0.0000047123456789", "4.7123456789 µH"),
        (Quantity::Length, "0.1", "100 mm"),
        (Quantity::Ratio, "0.05", "0.05"),
    ] {
        assert_eq!(display_quantity(value, quantity.unit()), shown);
        assert_eq!(quantity.parse(shown).unwrap(), CanonicalQuantity::new(value, quantity.unit()).unwrap(), "{shown}");
    }
    assert_eq!(display_quantity("01", "V"), "01 V");
}

/// New prefix boundaries round-trip exactly, including exa next to scientific notation
/// and base-unit letters that must not be mistaken for case-sensitive prefix aliases.
#[test]
fn atto_through_exa_inputs_preserve_precision_and_scientific_notation() {
    for (quantity, input, value, shown) in [
        (Quantity::Capacitance, "1aF", "0.000000000000000001", "1 aF"),
        (Quantity::Capacitance, "999aF", "0.000000000000000999", "999 aF"),
        (Quantity::Capacitance, "1000aF", "0.000000000000001", "1 fF"),
        (Quantity::Capacitance, "1.001fF", "0.000000000000001001", "1.001 fF"),
        (Quantity::Capacitance, "999.999fF", "0.000000000000999999", "999.999 fF"),
        (Quantity::Capacitance, "1000fF", "0.000000000001", "1 pF"),
        (Quantity::Capacitance, "-1f", "-0.000000000000001", "-1 fF"),
        (Quantity::Capacitance, "1F", "1", "1 F"),
        (Quantity::Current, "1A", "1", "1 A"),
        (Quantity::Current, "1aA", "0.000000000000000001", "1 aA"),
        (Quantity::Frequency, "999.999THz", "999999000000000", "999.999 THz"),
        (Quantity::Frequency, "1000THz", "1000000000000000", "1 PHz"),
        (Quantity::Frequency, "999.999PHz", "999999000000000000", "999.999 PHz"),
        (Quantity::Frequency, "1000PHz", "1000000000000000000", "1 EHz"),
        (Quantity::Frequency, "1E", "1000000000000000000", "1 EHz"),
        (Quantity::Frequency, "-1EHz", "-1000000000000000000", "-1 EHz"),
        (Quantity::Frequency, "1E3 Hz", "1000", "1 kHz"),
        (Quantity::Frequency, "1E+3Hz", "1000", "1 kHz"),
        (Quantity::Frequency, "1E-3Hz", "0.001", "1 mHz"),
        (Quantity::Frequency, "1e18Hz", "1000000000000000000", "1 EHz"),
        (Quantity::Frequency, "1e-3EHz", "1000000000000000", "1 PHz"),
        (Quantity::Resistance, "1ER", "1000000000000000000", "1 EΩ"),
        (Quantity::Capacitance, "0fF", "0", "0 F"),
    ] {
        let expected = CanonicalQuantity::new(value, quantity.unit()).unwrap();
        assert_eq!(quantity.parse(input).unwrap(), expected, "{input}");
        assert_eq!(expected.display(), shown, "{input}");
        assert_eq!(quantity.parse(shown).unwrap(), expected, "{shown}");
    }
    for input in ["0.1aF", "1.0001fF", "1e-19F", "1AF", "1FF", "1zF", "1QF", "1EE", "1E+", "1e", "1eHz"] {
        assert!(Quantity::Capacitance.parse(input).is_err(), "{input}");
    }
    let maximum = CanonicalQuantity::new("999999999999999999999999999999999999", "Hz").unwrap();
    assert_eq!(maximum.display(), "999999999999999999.999999999999999999 EHz");
    assert_eq!(Quantity::Frequency.parse(&maximum.display()).unwrap(), maximum);
}
