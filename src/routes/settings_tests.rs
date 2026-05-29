use super::*;

/// Trust-boundary: empty / whitespace-only input maps to `None` so
/// the TTS layer uses the endpoint's own default speed.
#[test]
fn parse_tts_speed_empty_or_whitespace_is_none_so_endpoint_default_is_used() {
    assert_eq!(parse_tts_speed("").expect("empty"), None);
    assert_eq!(parse_tts_speed("   ").expect("ws"), None);
    assert_eq!(parse_tts_speed("\t \n").expect("mixed-ws"), None);
}

/// Trust-boundary: a numeric value inside the band parses to `Some`.
#[test]
fn parse_tts_speed_accepts_in_range_values_including_both_endpoints() {
    assert_eq!(parse_tts_speed("1.0").expect("one"), Some(1.0));
    assert_eq!(parse_tts_speed("0.25").expect("min"), Some(TTS_SPEED_MIN));
    assert_eq!(parse_tts_speed("4.0").expect("max"), Some(TTS_SPEED_MAX));
}

/// Trust-boundary: non-numeric input must error before the value
/// reaches OpenRouter (which would charge for a 4xx round trip).
#[test]
fn parse_tts_speed_rejects_non_numeric_input_before_it_reaches_openrouter() {
    let err = parse_tts_speed("abc").expect_err("non-numeric");
    assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("number")));
    let err = parse_tts_speed("1.0x").expect_err("trailing chars");
    assert!(matches!(err, AppError::BadRequest(_)));
}

/// Trust-boundary: values outside the OpenAI-supported band must
/// error before reaching the endpoint — including just-below-min,
/// just-above-max, and grossly out-of-range.
#[test]
fn parse_tts_speed_rejects_out_of_range_values_at_both_edges_and_grossly_outside() {
    for bad in ["0.24", "4.01", "0", "-1.0", "9.0", "100"] {
        let err = parse_tts_speed(bad).expect_err(&format!("expected reject for `{bad}`"));
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}
