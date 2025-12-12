use memmap2::Mmap;
use std::fs::File;
use std::path::PathBuf;
use anyhow::Result;
use encoding_rs::{Encoding, UTF_8, UTF_16LE, UTF_16BE, WINDOWS_1252};

pub struct FileReader {
    mmap: Mmap,
    path: PathBuf,
    encoding: &'static Encoding,
}

impl FileReader {
    pub fn new(path: PathBuf, encoding: &'static Encoding) -> Result<Self> {
        let file = File::open(&path)?;
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
