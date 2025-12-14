use anyhow::Result;
use regex::bytes::Regex;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
    Arc,
};

pub enum ReplaceMessage {
    Progress(usize, usize), // bytes_processed, total_bytes
    Done,
    Error(String),
}

pub struct Replacer;

impl Replacer {
    pub fn replace_single(
        file_path: &Path,
        offset: usize,
        old_len: usize,
        new_text: &str,
    ) -> Result<()> {
        let new_bytes = new_text.as_bytes();

        if new_bytes.len() == old_len {
            // In-place optimization
            let mut file = OpenOptions::new().write(true).open(file_path)?;
            file.seek(SeekFrom::Start(offset as u64))?;
            file.write_all(new_bytes)?;
            return Ok(());
        }

        // Different length: rewrite file
        let temp_path = file_path.with_extension("tmp");
        {
            let mut input_file = File::open(file_path)?;
            let mut output_file = BufWriter::new(File::create(&temp_path)?);

            // Copy before match
            let mut buffer = vec![0u8; 8192];
            let mut remaining = offset as u64;

            while remaining > 0 {
                let to_read = std::cmp::min(remaining, buffer.len() as u64) as usize;
                let n = input_file.read(&mut buffer[0..to_read])?;
                if n == 0 {
                    break; // Unexpected EOF
                }
                output_file.write_all(&buffer[0..n])?;
                remaining -= n as u64;
            }

            // Write replacement
            output_file.write_all(new_bytes)?;

            // Skip old match in input
            input_file.seek(SeekFrom::Current(old_len as i64))?;

            // Copy rest
            std::io::copy(&mut input_file, &mut output_file)?;
        }

        // Replace original file
        // On Windows, rename might fail if target exists.
        if std::fs::rename(&temp_path, file_path).is_err() {
            // Try to remove target and rename again.
            if std::fs::remove_file(file_path).is_ok() {
                std::fs::rename(&temp_path, file_path)?;
            } else {
                return Err(anyhow::anyhow!(
                    "Failed to replace file. It might be open by another process."
                ));
            }
        }

        Ok(())
    }

    pub fn replace_all(
        input_path: &Path,
        output_path: &Path,
        query: &str,
        replace_with: &str,
        use_regex: bool,
        tx: Sender<ReplaceMessage>,
        cancel_token: Arc<AtomicBool>,
    ) {
        match Self::replace_all_inner(
            input_path,
            output_path,
            query,
            replace_with,
            use_regex,
            &tx,
            cancel_token,
        ) {
            Ok(_) => {
                let _ = tx.send(ReplaceMessage::Done);
            }
            Err(e) => {
                let _ = tx.send(ReplaceMessage::Error(e.to_string()));
            }
        }
    }

    fn replace_all_inner(
        input_path: &Path,
        output_path: &Path,
        query: &str,
        replace_with: &str,
        use_regex: bool,
        tx: &Sender<ReplaceMessage>,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<()> {
        let mut input_file = File::open(input_path)?;
        let file_len = input_file.metadata()?.len() as usize;
        let mut output_file = BufWriter::new(File::create(output_path)?);

        let regex = if use_regex {
            Regex::new(query)?
        } else {
            let pattern = format!("(?i){}", regex::escape(query));
            Regex::new(&pattern)?
        };

        let replace_with_bytes = replace_with.as_bytes();

        // Buffer size: 1MB
        const BUFFER_SIZE: usize = 1024 * 1024;
        // Overlap: enough to cover max match length.
        const OVERLAP_SIZE: usize = 4096;

        let mut buffer = vec![0u8; BUFFER_SIZE + OVERLAP_SIZE];
        let mut eof = false;

        // Initial fill
        let n = input_file.read(&mut buffer[0..BUFFER_SIZE])?;
        let mut buffer_len = n;
        if n < BUFFER_SIZE {
            eof = true;
        }

        let mut processed_offset = 0;

        while buffer_len > 0 {
            if cancel_token.load(Ordering::Relaxed) {
                return Ok(());
            }

            // Ensure we end at a char boundary to avoid splitting UTF-8 chars
            // even though we use bytes regex, we want to respect text boundaries if possible.
            let mut valid_len = buffer_len;
            while valid_len > 0 && !is_utf8_char_boundary(buffer[valid_len]) {
                valid_len -= 1;
            }
            if valid_len == 0 && buffer_len > 0 {
                valid_len = buffer_len;
            }

            let chunk_bytes = &buffer[..valid_len];

            let safe_zone_end = if eof {
                valid_len
            } else {
                valid_len.saturating_sub(OVERLAP_SIZE)
            };

            let mut last_match_end = 0;

            for cap in regex.captures_iter(chunk_bytes) {
                let mat = cap.get(0).unwrap();
                if mat.start() >= safe_zone_end {
                    break;
                }

                // Write text before match
                output_file.write_all(&chunk_bytes[last_match_end..mat.start()])?;

                // Expand replacement
                let mut dst = Vec::new();
                cap.expand(replace_with_bytes, &mut dst);
                output_file.write_all(&dst)?;

                last_match_end = mat.end();
            }

            // If last_match_end > safe_zone_end, it means we processed a match that crossed the boundary.
            // In that case, we have already written the replacement, so we should NOT write the original text.
            // And we should shift the buffer starting from last_match_end.

            let shift_start = if last_match_end > safe_zone_end {
                last_match_end
            } else {
                // Write remaining text in safe zone
                output_file.write_all(&chunk_bytes[last_match_end..safe_zone_end])?;
                safe_zone_end
            };

            // Shift remaining bytes to start
            let remaining_bytes = &buffer[shift_start..buffer_len];
            let remaining_len = remaining_bytes.len();

            let remaining_vec = remaining_bytes.to_vec();
            buffer[0..remaining_len].copy_from_slice(&remaining_vec);

            // Fill the rest of the buffer
            if !eof {
                let bytes_to_read = BUFFER_SIZE - remaining_len;
                let n =
                    input_file.read(&mut buffer[remaining_len..remaining_len + bytes_to_read])?;
                buffer_len = remaining_len + n;
                if n == 0 {
                    eof = true;
                }
            } else {
                buffer_len = remaining_len; // We keep the remaining bytes if EOF (should be 0 if we processed everything)
                if remaining_len == 0 {
                    buffer_len = 0;
                }
            }

            processed_offset += shift_start;
            let _ = tx.send(ReplaceMessage::Progress(processed_offset, file_len));
        }

        output_file.flush()?;
        Ok(())
    }
}

fn is_utf8_char_boundary(b: u8) -> bool {
    // In UTF-8, continuation bytes start with 10xxxxxx (0x80 to 0xBF)
    // So a byte is a char boundary if it is NOT a continuation byte.
    // i.e. it is < 0x80 or >= 0xC0.
    (b as i8) >= -0x40
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc;
    use tempfile::NamedTempFile;

    #[test]
    fn test_replace_all_simple() -> Result<()> {
        let mut input = NamedTempFile::new()?;
        write!(input, "Hello World, Hello Universe")?;
        let input_path = input.path().to_path_buf();

        let output = NamedTempFile::new()?;
        let output_path = output.path().to_path_buf();

        let (tx, rx) = mpsc::channel();
        let cancel_token = Arc::new(AtomicBool::new(false));

        Replacer::replace_all(
            &input_path,
            &output_path,
            "Hello",
            "Hi",
            false,
            tx,
            cancel_token,
        );

        // Wait for done
        loop {
            match rx.recv() {
                Ok(ReplaceMessage::Done) => break,
                Ok(ReplaceMessage::Error(e)) => panic!("Error: {}", e),
                Ok(ReplaceMessage::Progress(_, _)) => continue,
                Err(_) => break,
            }
        }

        let content = std::fs::read_to_string(&output_path)?;
        assert_eq!(content, "Hi World, Hi Universe");
        Ok(())
    }

    #[test]
    fn test_replace_all_regex() -> Result<()> {
        let mut input = NamedTempFile::new()?;
        write!(input, "Item 1, Item 2, Item 3")?;
        let input_path = input.path().to_path_buf();

        let output = NamedTempFile::new()?;
        let output_path = output.path().to_path_buf();

        let (tx, rx) = mpsc::channel();
        let cancel_token = Arc::new(AtomicBool::new(false));

        Replacer::replace_all(
            &input_path,
            &output_path,
            r"Item (\d)",
            "Object $1",
            true,
            tx,
            cancel_token,
        );

        loop {
            match rx.recv() {
                Ok(ReplaceMessage::Done) => break,
                Ok(ReplaceMessage::Error(e)) => panic!("Error: {}", e),
                Ok(ReplaceMessage::Progress(_, _)) => continue,
                Err(_) => break,
            }
        }

        let content = std::fs::read_to_string(&output_path)?;
        assert_eq!(content, "Object 1, Object 2, Object 3");
        Ok(())
    }
}
