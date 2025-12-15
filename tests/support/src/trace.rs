pub use tracing;
use tracing::Level;
use tracing::level_filters::LevelFilter;
pub use tracing_subscriber;

use tracing_subscriber::filter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn init() -> tracing::dispatcher::DefaultGuard {
    let filter = filter::Targets::new()
        .with_default(Level::TRACE) // Default level INFO
        .with_target("two_plz::hpack", LevelFilter::OFF)
        .with_target("two_plz::codec", LevelFilter::OFF);
    //.with_target("two_plz::frame", LevelFilter::OFF);
    let use_colors = atty::is(atty::Stream::Stdout);
    let layer = tracing_tree::HierarchicalLayer::default()
        .with_writer(tracing_subscriber::fmt::writer::TestWriter::default())
        .with_indent_lines(true)
        .with_ansi(use_colors)
        .with_targets(true)
        .with_indent_amount(2);

    tracing_subscriber::registry()
        .with(layer)
        .with(filter)
        .set_default()
}
