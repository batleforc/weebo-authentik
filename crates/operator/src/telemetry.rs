use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

/// Always initializes plain `tracing`; layers an OTLP trace exporter and
/// installs a global OTLP metrics `MeterProvider` only when
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Real, not a stub — Otel is the
/// stated observability stack in `.prompt/init.md` and this part is
/// genuinely cheap to finish now (unlike the reconcile business logic).
///
/// Every `opentelemetry::global::meter(...)` call made by
/// `adapters-inbound`'s reconcile-loop/webhook metrics (see
/// `adapters_inbound::controller::metrics`) is safe to make unconditionally
/// regardless of whether this branch runs — the global API is a no-op
/// recorder until a real `MeterProvider` is installed, same facade
/// pattern as `tracing`'s macros being safe to call before a subscriber
/// is set.
pub fn init() -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer();

    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
        let span_exporter = SpanExporter::builder().with_tonic().build()?;
        let tracer_provider = SdkTracerProvider::builder()
            .with_batch_exporter(span_exporter)
            .build();
        let tracer = opentelemetry::trace::TracerProvider::tracer(
            &tracer_provider,
            "weebo-authentik-operator",
        );
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        Registry::default()
            .with(env_filter)
            .with(fmt_layer)
            .with(otel_layer)
            .try_init()?;

        let metric_exporter = MetricExporter::builder().with_tonic().build()?;
        let meter_provider = SdkMeterProvider::builder()
            .with_periodic_exporter(metric_exporter)
            .build();
        opentelemetry::global::set_meter_provider(meter_provider);
    } else {
        Registry::default()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()?;
    }

    Ok(())
}
