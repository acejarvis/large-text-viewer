use anyhow::Result;
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252};
use memmap2::Mmap;
use std::fs::File;
use std::path::PathBuf;

pub struct FileReader {
    mmap: Mmap,
    path: PathBuf,
    encoding: &'static Encoding,
}

impl FileReader {
    pub fn new(path: PathBuf, encoding: &'static Encoding) -> Result<Self> {
        let file = File::open(&path)?;
        let metadata = file.metadata()?;
        if metadata.len() == 0 {
            anyhow::bail!("Cannot memory-map an empty file: {:?}", path);
        }
        let mmap = unsafe { Mmap::map(&file)? };

        Ok(Self {
            mmap,
            path,
            encoding,
        })
    }

    pub fn get_chunk(&self, start: usize, end: usize) -> String {
        let end = end.min(self.mmap.len());
        if start >= end {
            return String::new();
        }

        let bytes = &self.mmap[start..end];
        let (cow, _encoding, _had_errors) = self.encoding.decode(bytes);
        cow.into_owned()
    }

    pub fn get_bytes(&self, start: usize, end: usize) -> &[u8] {
        let end = end.min(self.mmap.len());
        if start >= end {
            return &[];
        }
        &self.mmap[start..end]
    }

    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn encoding(&self) -> &'static Encoding {
        self.encoding
    }

    pub fn all_data(&self) -> &[u8] {
        &self.mmap[..]
    }
}

pub fn detect_encoding(bytes: &[u8]) -> &'static Encoding {
    // Check for BOM
    if bytes.len() >= 3 && bytes[0..3] == [0xEF, 0xBB, 0xBF] {
        return UTF_8;
    }
    if bytes.len() >= 2 {
        if bytes[0..2] == [0xFF, 0xFE] {
            return UTF_16LE;
        }
        if bytes[0..2] == [0xFE, 0xFF] {
            return UTF_16BE;
        }
    }

    // Try UTF-8 validation
    if std::str::from_utf8(bytes).is_ok() {
        return UTF_8;
    }

    // Default to WINDOWS_1252 (similar to ISO-8859-1)
    WINDOWS_1252
}

pub fn available_encodings() -> Vec<(&'static str, &'static Encoding)> {
    vec![
        ("UTF-8", UTF_8),
        ("UTF-16 LE", UTF_16LE),
        ("UTF-16 BE", UTF_16BE),
        ("Windows-1252", WINDOWS_1252),
        ("ISO-8859-1", encoding_rs::WINDOWS_1252), // Similar enough
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_detect_encoding() {
        assert_eq!(detect_encoding(b"\xEF\xBB\xBFhello"), UTF_8);
        assert_eq!(detect_encoding(b"\xFF\xFEhello"), UTF_16LE);
        assert_eq!(detect_encoding(b"\xFE\xFFhello"), UTF_16BE);
        assert_eq!(detect_encoding(b"hello world"), UTF_8);
        // Invalid UTF-8 sequence
        assert_eq!(detect_encoding(b"\xFF\xFF\xFF"), WINDOWS_1252);
    }

    #[test]
    fn test_file_reader() -> Result<()> {
        let mut file = NamedTempFile::new()?;
        write!(file, "Hello World\nLine 2")?;
        let path = file.path().to_path_buf();

        let reader = FileReader::new(path.clone(), UTF_8)?;
        assert_eq!(reader.len(), 18);
        assert_eq!(reader.get_chunk(0, 5), "Hello");
        assert_eq!(reader.get_chunk(6, 11), "World");
        assert_eq!(reader.get_bytes(0, 5), b"Hello");
        
        Ok(())
    }

    #[test]
    fn test_empty_file() -> Result<()> {
        let file = NamedTempFile::new()?;
        let path = file.path().to_path_buf();
        let result = FileReader::new(path, UTF_8);
        assert!(result.is_err());
        Ok(())
    }
}
