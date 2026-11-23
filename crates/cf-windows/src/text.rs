//! Text in the form Windows calls take: UTF-16, ended by a zero.

/// `s` as a zero-ended UTF-16 string, for passing to a Windows call.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The text in a buffer a Windows call filled: up to the first zero, or all
/// of it. Invalid UTF-16 is replaced, never an error.
pub(crate) fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_stops_at_the_first_zero() {
        let w = wide("Café ✓");
        assert_eq!(w.last(), Some(&0));
        assert_eq!(from_wide(&w), "Café ✓");
        assert_eq!(from_wide(&[0x41, 0, 0x42]), "A");
        assert_eq!(from_wide(&[0x41, 0x42]), "AB");
        assert_eq!(from_wide(&[]), "");
    }
}
