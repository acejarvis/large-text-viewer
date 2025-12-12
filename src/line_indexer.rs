use crate::file_reader::FileReader;

pub struct LineIndexer {
    line_offsets: Vec<usize>,
    total_lines: usize,
    indexed: bool,
}

impl LineIndexer {
    pub fn new() -> Self {
        Self {
            line_offsets: vec![0],
            total_lines: 0,
            indexed: false,
        }
    }

    pub fn index_file(&mut self, reader: &FileReader) {
        self.line_offsets.clear();
        self.line_offsets.push(0);

        let data = reader.all_data();
        let file_size = data.len();

        // For very large files, sample every N bytes to build approximate index
        const SAMPLE_THRESHOLD: usize = 100_000_000; // 100 MB
        
        if file_size > SAMPLE_THRESHOLD {
            // Sample-based indexing for very large files
            self.sample_index(data, file_size);
        } else {
            // Full indexing for smaller files
            self.full_index(data);
        }

        self.total_lines = self.line_offsets.len();
        self.indexed = true;
    }

    fn full_index(&mut self, data: &[u8]) {
        for (i, &byte) in data.iter().enumerate() {
            if byte == b'\n' {
                self.line_offsets.push(i + 1);
            }
        }
    }

    fn sample_index(&mut self, data: &[u8], file_size: usize) {
        // Sample every 1MB
        const SAMPLE_INTERVAL: usize = 1_000_000;
        let mut pos = 0;

        while pos < file_size {
            // Find newlines in this chunk
            let end = (pos + SAMPLE_INTERVAL).min(file_size);
            for i in pos..end {
                if data[i] == b'\n' {
                    self.line_offsets.push(i + 1);
                }
            }
            pos = end;
        }
    }

    pub fn get_line_offset(&self, line_num: usize) -> Option<usize> {
        if line_num >= self.line_offsets.len() {
            return None;
        }
        Some(self.line_offsets[line_num])
    }

    pub fn get_line_range(&self, line_num: usize) -> Option<(usize, usize)> {
        if line_num >= self.line_offsets.len() {
            return None;
        }
        
        let start = self.line_offsets[line_num];
        let end = if line_num + 1 < self.line_offsets.len() {
            self.line_offsets[line_num + 1]
        } else {
            usize::MAX
        };
        
        Some((start, end))
    }

    pub fn find_line_at_offset(&self, offset: usize) -> usize {
        match self.line_offsets.binary_search(&offset) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1),
        }
    }

    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    pub fn is_indexed(&self) -> bool {
        self.indexed
    }
}
