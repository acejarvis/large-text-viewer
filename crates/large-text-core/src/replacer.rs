use anyhow::Result;
use regex::bytes::Regex;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
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
