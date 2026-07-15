use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

/// Always initializes plain `tracing`; layers an OTLP exporter on top only
/// when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Real, not a stub — Otel is
/// the stated observability stack in `.prompt/init.md` and this part is
/// genuinely cheap to finish now (unlike the reconcile business logic).
pub fn init() -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer();

    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
        let exporter = SpanExporter::builder().with_tonic().build()?;
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();
        let tracer =
            opentelemetry::trace::TracerProvider::tracer(&provider, "weebo-authentik-operator");
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        Registry::default()
            .with(env_filter)
            .with(fmt_layer)
            .with(otel_layer)
            .try_init()?;
    } else {
        Registry::default()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()?;
    }

    Ok(())
}
