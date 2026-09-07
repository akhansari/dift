use base64::{display::Base64Display, engine::general_purpose::STANDARD};
use std::io::{self, Write};

/// Write UTF-8 source text to the terminal clipboard using OSC 52.
pub(super) fn copy(output: &mut impl Write, text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let sequence = format!(
        "\x1b]52;c;{}\x1b\\",
        Base64Display::new(text.as_bytes(), &STANDARD)
    );
    output.write_all(sequence.as_bytes())?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_selection_leaves_clipboard_untouched() -> io::Result<()> {
        let mut output = Vec::new();
        copy(&mut output, "")?;
        assert!(output.is_empty());
        Ok(())
    }

    #[test]
    fn one_byte_uses_two_padding_characters() -> io::Result<()> {
        let mut output = Vec::new();
        copy(&mut output, "f")?;
        assert_eq!(output, b"\x1b]52;c;Zg==\x1b\\");
        Ok(())
    }

    #[test]
    fn two_bytes_use_one_padding_character() -> io::Result<()> {
        let mut output = Vec::new();
        copy(&mut output, "fo")?;
        assert_eq!(output, b"\x1b]52;c;Zm8=\x1b\\");
        Ok(())
    }

    #[test]
    fn complete_triplet_needs_no_padding() -> io::Result<()> {
        let mut output = Vec::new();
        copy(&mut output, "foo")?;
        assert_eq!(output, b"\x1b]52;c;Zm9v\x1b\\");
        Ok(())
    }

    #[test]
    fn unicode_and_source_control_characters_are_encoded() -> io::Result<()> {
        let mut output = Vec::new();
        copy(&mut output, "é\t\n\x1b")?;
        assert_eq!(output, b"\x1b]52;c;w6kJChs=\x1b\\");
        Ok(())
    }
}
