use std::io::{self, Write};

/// Write UTF-8 source text to the terminal clipboard using OSC 52.
pub(super) fn copy(output: &mut impl Write, text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut sequence = Vec::with_capacity(9 + text.len().div_ceil(3) * 4);
    sequence.extend_from_slice(b"\x1b]52;c;");
    for chunk in text.as_bytes().chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        sequence.extend_from_slice(&[
            ALPHABET[usize::from(a >> 2)],
            ALPHABET[usize::from((a & 3) << 4 | b >> 4)],
            if chunk.len() > 1 {
                ALPHABET[usize::from((b & 15) << 2 | c >> 6)]
            } else {
                b'='
            },
            if chunk.len() > 2 {
                ALPHABET[usize::from(c & 63)]
            } else {
                b'='
            },
        ]);
    }
    sequence.extend_from_slice(b"\x1b\\");
    output.write_all(&sequence)?;
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
