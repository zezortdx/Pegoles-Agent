//! Classification of model-supplied text (hostile by assumption).
//!
//! Shared so every layer that judges or shows untrusted text draws the
//! same line: Policy denies typed text containing these characters for
//! every planner, whichever provider produced it.

/// Characters that change how text DISPLAYS without being visible
/// themselves: bidi embeddings, overrides, isolates and marks, zero-width
/// spaces and joiners, the word joiner and invisible operators, the byte
/// order mark, every other Unicode format character (category Cf, complete
/// as of Unicode 16), the default-ignorable fillers, variation selectors
/// and tags, and the line/paragraph separators. Text holding any of them
/// can read differently in the activity feed than what reaches the guest.
///
/// C0/C1 controls are category Cc: callers check `char::is_control`
/// alongside this function.
pub fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        // Category Cf.
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            // U+2060..U+2064 and U+2066..U+206F are Cf; U+2065 is a
            // reserved default-ignorable in the same block.
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF0}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            // Tags (Cf), variation selectors supplement and reserved
            // default-ignorables.
            | '\u{E0000}'..='\u{E0FFF}'
            // Default-ignorable, not Cf: rendered as nothing.
            | '\u{034F}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180D}'
            | '\u{180F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FFA0}'
            // Line and paragraph separators (Zl/Zp): invisible breaks.
            | '\u{2028}'..='\u{2029}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_bidi_zero_width_and_other_format_characters() {
        // Bidi embeddings/overrides, isolates and marks.
        let bidi = "\u{202A}\u{202B}\u{202C}\u{202D}\u{202E}\u{2066}\u{2067}\u{2068}\u{2069}\u{200E}\u{200F}\u{061C}";
        // Zero width, word joiner, invisible operators, byte order mark.
        let zero_width = "\u{200B}\u{200C}\u{200D}\u{2060}\u{2061}\u{2062}\u{2063}\u{2064}\u{FEFF}";
        // Other Cf, default-ignorables and line/paragraph separators.
        let other = "\u{00AD}\u{180E}\u{0600}\u{FFF9}\u{E0001}\u{E0041}\u{E007F}\u{1D173}\u{13430}\u{1BCA0}\u{034F}\u{3164}\u{FE0F}\u{E0100}\u{2028}\u{2029}";
        for c in bidi.chars().chain(zero_width.chars()).chain(other.chars()) {
            assert!(is_invisible_format(c), "{:04X}", c as u32);
        }
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for c in
            "Pegoles is alive: ção 漢字 é ß ñ Ω → 🙂 \n\t-_=+[]{};:'\",.<>/?`~!@#$%^&*()".chars()
        {
            assert!(!is_invisible_format(c), "{:04X}", c as u32);
        }
    }
}
