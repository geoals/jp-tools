use super::*;

#[test]
fn format_seconds_formats_correctly() {
    assert_eq!(format_seconds(0.0), "0:00");
    assert_eq!(format_seconds(5.5), "0:05");
    assert_eq!(format_seconds(65.0), "1:05");
    assert_eq!(format_seconds(3661.0), "61:01");
}
