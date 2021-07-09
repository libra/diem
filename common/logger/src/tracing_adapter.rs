// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate as dl;
use std::fmt;
use tracing as tr;
use tracing_subscriber::{layer::Context, Layer};

/// A layer that translates tracing events into diem-logger events.
pub struct TracingToDiemLoggerLayer;

fn translate_level(level: &tr::Level) -> Option<dl::Level> {
    if *level == tr::Level::ERROR {
        return Some(dl::Level::Error);
    }
    if *level == tr::Level::INFO {
        return Some(dl::Level::Info);
    }
    if *level == tr::Level::DEBUG {
        return Some(dl::Level::Debug);
    }
    if *level == tr::Level::TRACE {
        return Some(dl::Level::Trace);
    }
    if *level == tr::Level::WARN {
        return Some(dl::Level::Warn);
    }
    None
}

fn translate_metadata(metadata: &tr::Metadata<'static>) -> Option<dl::Metadata> {
    let level = translate_level(metadata.level())?;

    Some(dl::Metadata::new(
        level,
        metadata.target(),
        metadata.module_path().unwrap_or(""),
        metadata.file().unwrap_or(""),
        metadata.line().unwrap_or(0),
        "",
    ))
}

struct KeyValueVisitorAdapter<'a> {
    visitor: &'a mut dyn dl::Visitor,
}

impl<'a> tr::field::Visit for KeyValueVisitorAdapter<'a> {
    fn record_debug(&mut self, field: &tr::field::Field, value: &dyn fmt::Debug) {
        self.visitor
            .visit_pair(dl::Key::new(field.name()), dl::Value::Debug(value))
    }
}

struct EventKeyValueAdapter<'a, 'b> {
    event: &'a tr::Event<'b>,
}

impl<'a, 'b> dl::Schema for EventKeyValueAdapter<'a, 'b> {
    fn visit(&self, visitor: &mut dyn dl::Visitor) {
        self.event.record(&mut KeyValueVisitorAdapter { visitor })
    }
}

impl<S: tr::Subscriber> Layer<S> for TracingToDiemLoggerLayer {
    fn on_event(&self, event: &tr::Event, _ctx: Context<S>) {
        let metadata = match translate_metadata(event.metadata()) {
            Some(metadata) => metadata,
            None => {
                dl::warn!(
                    "[tracing-to-diem-logger] failed to translate event due to unknown level {:?}",
                    event.metadata().level()
                );
                return;
            }
        };

        // `tracing::Event` contains an implicit field named "message".
        // However I couldn't figure out a way to convert it to `fmt::Arguments` due to lifetime issues.
        // Therefore I'm omitting message argument to `Event::dispatch`.
        // This should generally be fine since the message will be translated as a normal record.
        if dl::logger::enabled(&metadata) {
            dl::Event::dispatch(&metadata, None, &[&EventKeyValueAdapter { event }]);
        }
    }
}
