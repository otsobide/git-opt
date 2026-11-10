use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    model::CodecOptions,
    values::{ByteSize, FileFormat, OptimizerId, Profile, Savings},
};

/// Strategy for one family of file formats: produce a smaller candidate and
/// prove it is equivalent to the original. One implementation per library
/// (oxipng, mozjpeg, libwebp…); replace a library by registering another
/// strategy for the same formats.
#[async_trait]
pub trait FormatStrategy: Send + Sync {
    /// Stable slug used in settings, cache keys and reports.
    fn id(&self) -> &'static str;
    /// Human-friendly name for the UI.
    fn label(&self) -> &'static str;
    fn formats(&self) -> &'static [FileFormat];
    fn enabled_by_default(&self) -> bool {
        true
    }
    /// One Markdown line for the pull request: what changes and what is kept.
    fn explanation(&self) -> &'static str;
    /// Everything that changes the output for a given input (cache key).
    fn profile(&self, options: &CodecOptions) -> Profile;
    /// `Ok(detail)` when the strategy can run here, `Err(reason)` otherwise.
    async fn probe(&self) -> Result<String, String>;
    /// Produces a candidate; it may be bigger than the input.
    async fn optimize(&self, input: Arc<Vec<u8>>, options: &CodecOptions) -> anyhow::Result<Vec<u8>>;
    /// Must fail unless `candidate` is provably equivalent to `original`.
    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> anyhow::Result<()>;

    fn optimizer_id(&self) -> OptimizerId {
        OptimizerId::new(self.id()).expect("strategy ids are valid slugs")
    }
    fn handles(&self, format: FileFormat) -> bool {
        self.formats().contains(&format)
    }
}

/// Result of [`attempt`].
#[derive(Debug)]
pub enum Outcome {
    /// Smaller, worth it, and proven equivalent.
    Optimized(Vec<u8>),
    /// Nothing worth changing.
    Optimal(Option<String>),
    /// The strategy failed or could not prove equivalence.
    Failed(String),
}

/// The one way to optimize a file: optimize, keep the result only if the gain
/// passes the threshold, then require the strategy's proof of equivalence.
pub async fn attempt(strategy: &dyn FormatStrategy, input: Arc<Vec<u8>>, options: &CodecOptions) -> Outcome {
    let candidate = match strategy.optimize(input.clone(), options).await {
        Ok(candidate) => candidate,
        Err(e) => return Outcome::Failed(format!("{e:#}")),
    };
    let savings = Savings::new(ByteSize::new(input.len() as u64), ByteSize::new(candidate.len() as u64));
    if candidate.is_empty() {
        return Outcome::Optimal(None);
    }
    if let Err(reason) = options.threshold.check(&savings) {
        return Outcome::Optimal((reason != "no reduction").then_some(reason));
    }
    let candidate = Arc::new(candidate);
    if let Err(e) = strategy.verify(input, candidate.clone()).await {
        return Outcome::Failed(format!("lossless verification failed: {e:#}"));
    }
    Outcome::Optimized(Arc::try_unwrap(candidate).unwrap_or_else(|shared| (*shared).clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::SavingsThreshold;

    /// Halves the input; `verify` passes only if `honest`.
    struct Halver {
        honest: bool,
    }

    #[async_trait]
    impl FormatStrategy for Halver {
        fn id(&self) -> &'static str {
            "halver"
        }
        fn label(&self) -> &'static str {
            "Halver"
        }
        fn formats(&self) -> &'static [FileFormat] {
            &[FileFormat::Png]
        }
        fn explanation(&self) -> &'static str {
            "halves files"
        }
        fn profile(&self, _: &CodecOptions) -> Profile {
            Profile::new("halver")
        }
        async fn probe(&self) -> Result<String, String> {
            Ok("ok".into())
        }
        async fn optimize(&self, input: Arc<Vec<u8>>, _: &CodecOptions) -> anyhow::Result<Vec<u8>> {
            Ok(input[..input.len() / 2].to_vec())
        }
        async fn verify(&self, _: Arc<Vec<u8>>, _: Arc<Vec<u8>>) -> anyhow::Result<()> {
            anyhow::ensure!(self.honest, "pixels differ");
            Ok(())
        }
    }

    fn options(min_bytes: u64) -> CodecOptions {
        CodecOptions { threshold: SavingsThreshold::new(0.0, min_bytes), ..CodecOptions::default() }
    }

    #[tokio::test]
    async fn keeps_only_proven_worthwhile_results() {
        let input = Arc::new(vec![7u8; 1000]);
        assert!(
            matches!(attempt(&Halver { honest: true }, input.clone(), &options(1)).await, Outcome::Optimized(v) if v.len() == 500)
        );
        assert!(matches!(
            attempt(&Halver { honest: true }, input.clone(), &options(600)).await,
            Outcome::Optimal(Some(_))
        ));
        assert!(
            matches!(attempt(&Halver { honest: false }, input, &options(1)).await, Outcome::Failed(e) if e.contains("pixels differ"))
        );
    }
}
