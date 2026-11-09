use crate::values::BlobId;

/// What a format strategy concluded about a blob, for a given profile. Blob
/// ids are content hashes, so verdicts are valid in every repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobVerdict {
    /// Nothing worth changing.
    Optimal { detail: Option<String> },
    /// A verified smaller version exists (`blob`, once committed).
    Optimized { blob: Option<BlobId> },
    /// Could not be optimized or proven equivalent; not retried.
    Failed { reason: String },
}

impl BlobVerdict {
    /// Optimal and failed blobs are skipped; optimized ones are re-applied
    /// while their pull request is pending.
    pub fn skips_file(&self) -> bool {
        !matches!(self, Self::Optimized { .. })
    }
}
