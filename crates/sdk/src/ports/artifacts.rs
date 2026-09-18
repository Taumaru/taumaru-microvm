use std::future::Future;
use std::pin::Pin;

use crate::domain::registry::TaumaruRegistry;
use crate::error::SdkError;

pub(crate) type RegistryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, SdkError>> + Send + 'a>>;

pub(crate) trait ArtifactSource: Send + Sync {
    fn fetch_manifest(&self) -> RegistryFuture<'_, TaumaruRegistry>;
    fn fetch_file(&self, url: &str) -> RegistryFuture<'_, reqwest::Response>;
}
