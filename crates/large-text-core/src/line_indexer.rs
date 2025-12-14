use crate::file_reader::FileReader;

pub struct LineIndexer {
    line_offsets: Vec<usize>,
    total_lines: usize,
    indexed: bool,
    // Sparse sampling for large files
    sample_interval: usize,
    file_size: usize,
    avg_line_length: f64,
}

impl LineIndexer {
    pub fn new() -> Self {
        Self {
            line_offsets: vec![0],
            total_lines: 0,
            indexed: false,
            sample_interval: 0,
            file_size: 0,
            avg_line_length: 80.0,
        }
    }

    pub fn index_file(&mut self, reader: &FileReader) {
        self.line_offsets.clear();
        self.line_offsets.push(0);
        self.file_size = reader.len();

        // For small files (< 10MB), do full indexing
        // For large files, use sparse sampling only
        const FULL_INDEX_THRESHOLD: usize = 10_000_000; // 10 MB

        if self.file_size <= FULL_INDEX_THRESHOLD {
            // Full indexing for smaller files
            let data = reader.all_data();
            self.full_index(data);
            self.sample_interval = 0;
        } else {
            // Sparse sampling for large files - only sample at intervals
            self.sparse_sample_index(reader);
        }

        self.total_lines = if self.sample_interval > 0 {
            // Estimate total lines based on sampling
            self.estimate_total_lines()
        } else {
            self.line_offsets.len()
        };

        self.indexed = true;
    }

    fn full_index(&mut self, data: &[u8]) {
        for (i, &byte) in data.iter().enumerate() {
            if byte == b'\n' {
                self.line_offsets.push(i + 1);
            }
        }
    }

    fn sparse_sample_index(&mut self, reader: &FileReader) {
        // Only sample every 10MB for large files - creates sparse checkpoint index
        const SPARSE_SAMPLE_SIZE: usize = 10_000_000; // 10MB
        self.sample_interval = SPARSE_SAMPLE_SIZE;

        let mut pos = 0;
        let sample_count_limit = 100; // Limit to 100 samples max
        let mut sample_count = 0;

        let mut total_bytes_sampled = 0;
        let mut total_newlines_found = 0;

        // Sample a few chunks to estimate average line length
        while pos < self.file_size && sample_count < sample_count_limit {
            let chunk_end = (pos + SPARSE_SAMPLE_SIZE).min(self.file_size);
            let chunk = reader.get_bytes(pos, chunk_end);

            // Count newlines to estimate average line length
            // We limit the sampling to the first few chunks to avoid reading too much
            if sample_count < 5 {
                let newline_count = chunk.iter().filter(|&&b| b == b'\n').count();
                total_bytes_sampled += chunk.len();
                total_newlines_found += newline_count;
            }

            // Store checkpoint at this position
            self.line_offsets.push(pos);

            pos = chunk_end;
            sample_count += 1;
        }

        if total_newlines_found > 0 {
            self.avg_line_length = total_bytes_sampled as f64 / total_newlines_found as f64;
        }
    }

    fn estimate_total_lines(&self) -> usize {
        if self.avg_line_length > 0.0 {
            (self.file_size as f64 / self.avg_line_length) as usize
        } else {
            self.file_size / 80 // Assume 80 char average if unknown
        }
    }

    pub fn get_line_range(&self, line_num: usize) -> Option<(usize, usize)> {
        if self.sample_interval == 0 {
            // Full index available
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
        } else {
            // Sparse index - estimate line position
            // This will be resolved on-demand in get_line_with_reader
            let estimated_pos = (line_num as f64 * self.avg_line_length) as usize;
            Some((estimated_pos, usize::MAX))
        }
    }

    // Helper method to get actual line content by scanning from estimated position
    pub fn get_line_with_reader(
        &self,
        line_num: usize,
        reader: &FileReader,
    ) -> Option<(usize, usize)> {
        if self.sample_interval == 0 {
            // Use full index
            return self.get_line_range(line_num);
        }

        // For sparse index, estimate and scan
        let estimated_byte_pos = (line_num as f64 * self.avg_line_length) as usize;

        // Scan backwards to find start of line (in case we landed mid-line)
        // Increase scan radius to handle variance in line lengths and very long lines
        let scan_radius = (self.avg_line_length * 2.0).max(65536.0) as usize;
        let scan_start = estimated_byte_pos
            .saturating_sub(scan_radius)
            .min(self.file_size);
        let scan_end = (estimated_byte_pos + scan_radius).min(self.file_size);

        if scan_start >= scan_end {
            return None;
        }

        let chunk = reader.get_bytes(scan_start, scan_end);

        // Find newline before our estimated position
        let relative_est = estimated_byte_pos.saturating_sub(scan_start);
        let mut line_start = scan_start;
        let mut found_start = false;

        for i in (0..relative_est.min(chunk.len())).rev() {
            if chunk[i] == b'\n' {
                line_start = scan_start + i + 1;
                found_start = true;
                break;
            }
        }

        if !found_start {
            // If we didn't find a newline backwards, we might be in a very long line.
            // Fallback: just start at scan_start to ensure we show something.
            // This might start mid-line, but it guarantees the estimated position is visible.
            line_start = scan_start;
        }

        // Find newline after our position for line end
        let mut line_end = scan_end;
        let search_from = relative_est.min(chunk.len());

        for i in search_from..chunk.len() {
            if chunk[i] == b'\n' {
                line_end = scan_start + i;
                break;
            }
        }

        Some((line_start, line_end))
    }

    pub fn find_line_at_offset(&self, offset: usize) -> usize {
        if self.sample_interval == 0 {
            // Full index
            match self.line_offsets.binary_search(&offset) {
                Ok(line) => line,
                Err(line) => line.saturating_sub(1),
            }
        } else {
            // Sparse index - estimate
            if self.avg_line_length > 0.0 {
                (offset as f64 / self.avg_line_length) as usize
            } else {
                offset / 80
            }
        }
    }

    pub fn total_lines(&self) -> usize {
        self.total_lines
    }
}
