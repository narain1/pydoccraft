pub const MAX_MEMBERS: usize = 10_000;
pub const MAX_MEMBER_SIZE: u64 = 128 * 1024 * 1024;
pub const MAX_TOTAL_SIZE: u64 = 512 * 1024 * 1024;
pub const MAX_COMPRESSION_RATIO: u64 = 1_000;
pub const MAX_INPUT_SIZE: usize = 128 * 1024 * 1024;
pub const MAX_PAYLOAD_SIZE: usize = 64 * 1024 * 1024;

pub fn validate_input_size(size: usize) -> Result<(), crate::EngineError> {
    if size > MAX_INPUT_SIZE {
        return Err(crate::EngineError::ResourceLimit(
            "input exceeds 128 MiB".into(),
        ));
    }
    Ok(())
}

/// Stop JSON serialization before growing its output past the boundary budget.
pub fn bounded_json<T: serde::Serialize>(
    value: &T,
    maximum: usize,
) -> Result<String, crate::EngineError> {
    struct Output {
        bytes: Vec<u8>,
        maximum: usize,
        limited: bool,
        allocation_failed: bool,
    }
    impl std::io::Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let required = self.bytes.len().checked_add(bytes.len());
            let Some(required) = required.filter(|size| *size <= self.maximum) else {
                self.limited = true;
                return Err(std::io::Error::other(
                    "normalized JSON exceeds its byte budget",
                ));
            };
            if required > self.bytes.capacity() {
                let capacity = required
                    .max(self.bytes.capacity().saturating_mul(2))
                    .min(self.maximum);
                if let Err(error) = self.bytes.try_reserve_exact(capacity - self.bytes.len()) {
                    self.allocation_failed = true;
                    return Err(std::io::Error::other(error));
                }
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Output {
        bytes: Vec::with_capacity(maximum.min(4096)),
        maximum,
        limited: false,
        allocation_failed: false,
    };
    if let Err(error) = serde_json::to_writer(&mut output, value) {
        return Err(if output.limited {
            crate::EngineError::ResourceLimit(format!("normalized JSON exceeds {maximum} bytes"))
        } else if output.allocation_failed {
            crate::EngineError::ResourceLimit("cannot allocate bounded JSON output".into())
        } else {
            crate::EngineError::Malformed(format!("payload serialization failed: {error}"))
        });
    }
    String::from_utf8(output.bytes).map_err(|error| {
        crate::EngineError::Malformed(format!("invalid serialized UTF-8: {error}"))
    })
}

pub fn validate_member_count(count: usize) -> Result<(), crate::EngineError> {
    if count > MAX_MEMBERS {
        return Err(crate::EngineError::ResourceLimit(
            "archive has too many members".into(),
        ));
    }
    Ok(())
}

pub fn add_uncompressed_size(total: u64, size: u64) -> Result<u64, crate::EngineError> {
    let updated = total
        .checked_add(size)
        .ok_or_else(|| crate::EngineError::ResourceLimit("archive expansion overflow".into()))?;
    if updated > MAX_TOTAL_SIZE {
        return Err(crate::EngineError::ResourceLimit(
            "archive expands beyond aggregate limit".into(),
        ));
    }
    Ok(updated)
}

pub fn validate_member(name: &str, size: u64, compressed: u64) -> Result<(), crate::EngineError> {
    if name.starts_with('/')
        || name.starts_with('\\')
        || name.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(crate::EngineError::UnsafeArchive(
            "archive path traversal".into(),
        ));
    }
    if size > MAX_MEMBER_SIZE {
        return Err(crate::EngineError::ResourceLimit(
            "archive member is too large".into(),
        ));
    }
    if compressed == 0 {
        if size > 0 {
            return Err(crate::EngineError::UnsafeArchive(
                "suspicious compression ratio".into(),
            ));
        }
    } else if compressed
        .checked_mul(MAX_COMPRESSION_RATIO)
        .is_none_or(|limit| size > limit)
    {
        return Err(crate::EngineError::UnsafeArchive(
            "suspicious compression ratio".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_input_and_serialized_bytes_including_escapes() {
        assert!(validate_input_size(MAX_INPUT_SIZE).is_ok());
        assert!(validate_input_size(MAX_INPUT_SIZE + 1).is_err());
        assert_eq!(bounded_json(&"λ\n", 6).unwrap(), "\"λ\\n\"");
        assert!(matches!(
            bounded_json(&"λ\n", 5),
            Err(crate::EngineError::ResourceLimit(_))
        ));
        assert!(bounded_json(&vec!["a"; 100], 50).is_err());
        assert_eq!(bounded_json(&vec![1, 2], 5).unwrap(), "[1,2]");
    }

    #[test]
    fn rejects_unsafe_members() {
        assert!(validate_member("../escape", 1, 1).is_err());
        assert!(validate_member("/absolute", 1, 1).is_err());
        assert!(validate_member("word/document.xml", 10, 10).is_ok());
        assert!(validate_member("large", MAX_MEMBER_SIZE + 1, 1).is_err());
        assert!(validate_member_count(MAX_MEMBERS + 1).is_err());
        assert!(add_uncompressed_size(MAX_TOTAL_SIZE, 1).is_err());
        assert!(add_uncompressed_size(u64::MAX, 1).is_err());
    }

    #[test]
    fn checks_compression_ratio_without_truncation() {
        assert!(validate_member("exact-one", 1_000, 1).is_ok());
        assert!(validate_member("over-one", 1_001, 1).is_err());
        assert!(validate_member("exact-two", 2_000, 2).is_ok());
        assert!(validate_member("over-two", 2_001, 2).is_err());
        assert!(validate_member("zero", 1, 0).is_err());
        assert!(validate_member("overflow", u64::MAX, u64::MAX).is_err());
    }
}
