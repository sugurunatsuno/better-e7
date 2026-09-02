#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnexBUnit {
    pub data: Vec<u8>,
    pub nal_unit_type: u8,
}

#[derive(Debug, Default)]
pub struct AnnexBParser {
    buffer: Vec<u8>,
}

impl AnnexBParser {
    #[must_use]
    pub const fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<AnnexBUnit> {
        self.buffer.extend_from_slice(bytes);
        self.take_complete_units()
    }

    pub fn finish(&mut self) -> Option<AnnexBUnit> {
        let starts = find_start_codes(&self.buffer);
        let &(start, prefix_length) = starts.first()?;
        let header_index = start + prefix_length;
        if header_index >= self.buffer.len() {
            self.buffer.clear();
            return None;
        }
        let data = self.buffer.split_off(start);
        self.buffer.clear();
        Some(AnnexBUnit {
            nal_unit_type: data[prefix_length] & 0x1f,
            data,
        })
    }

    fn take_complete_units(&mut self) -> Vec<AnnexBUnit> {
        let starts = find_start_codes(&self.buffer);
        if starts.len() < 2 {
            return Vec::new();
        }

        let mut units = Vec::with_capacity(starts.len() - 1);
        for pair in starts.windows(2) {
            let (start, prefix_length) = pair[0];
            let (end, _) = pair[1];
            let header_index = start + prefix_length;
            if header_index < end {
                units.push(AnnexBUnit {
                    nal_unit_type: self.buffer[header_index] & 0x1f,
                    data: self.buffer[start..end].to_vec(),
                });
            }
        }

        let last_start = starts.last().map_or(0, |(start, _)| *start);
        self.buffer.drain(..last_start);
        units
    }
}

fn find_start_codes(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        if index + 4 <= bytes.len() && bytes[index..index + 4] == [0, 0, 0, 1] {
            starts.push((index, 4));
            index += 4;
        } else if bytes[index..index + 3] == [0, 0, 1] {
            starts.push((index, 3));
            index += 3;
        } else {
            index += 1;
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_units_across_chunk_boundaries() {
        let mut parser = AnnexBParser::new();
        assert!(parser.push(&[0, 0]).is_empty());
        assert!(parser.push(&[0, 1, 0x67, 1, 2, 0]).is_empty());

        let units = parser.push(&[0, 0, 1, 0x68, 3]);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].nal_unit_type, 7);
        assert_eq!(units[0].data, [0, 0, 0, 1, 0x67, 1, 2]);

        let last = parser.finish().unwrap();
        assert_eq!(last.nal_unit_type, 8);
        assert_eq!(last.data, [0, 0, 0, 1, 0x68, 3]);
    }

    #[test]
    fn ignores_bytes_before_the_first_start_code() {
        let mut parser = AnnexBParser::new();
        let units = parser.push(&[9, 9, 0, 0, 1, 0x65, 7, 0, 0, 1, 0x61]);

        assert_eq!(units.len(), 1);
        assert_eq!(units[0].nal_unit_type, 5);
        assert_eq!(units[0].data, [0, 0, 1, 0x65, 7]);
    }
}
