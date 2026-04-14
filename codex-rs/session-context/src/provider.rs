use crate::metrics::MetricsClient;
use tracing_subscriber::Layer;

#[derive(Debug, Clone, Default)]
pub struct OtelProvider;

impl OtelProvider {
    pub fn logger_layer<S>(&self) -> Option<Box<dyn Layer<S> + Send + Sync>>
    where
        S: tracing::Subscriber,
    {
        None
    }

    pub fn tracing_layer<S>(&self) -> Option<Box<dyn Layer<S> + Send + Sync>>
    where
        S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
    {
        None
    }

    pub fn metrics(&self) -> Option<&MetricsClient> {
        None
    }

    pub fn shutdown(&self) {}

    pub fn log_export_filter(_: &tracing::Metadata<'_>) -> bool {
        false
    }

    pub fn trace_export_filter(_: &tracing::Metadata<'_>) -> bool {
        false
    }
}
