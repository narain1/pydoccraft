pub const MAX_MEMBERS: usize = 10_000;
pub const MAX_MEMBER_SIZE: u64 = 128 * 1024 * 1024;
pub const MAX_TOTAL_SIZE: u64 = 512 * 1024 * 1024;
pub const MAX_COMPRESSION_RATIO: u64 = 1_000;

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
