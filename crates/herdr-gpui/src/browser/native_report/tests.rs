#![allow(clippy::unwrap_used)]
use super::*;
use std::sync::{Arc, Mutex, mpsc};
use tracing_subscriber::{Layer, prelude::*};

struct Capture(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{field}={value:?}"));
            }
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
}

#[test]
fn overflow_never_logs_token_urls_titles_or_page_bodies() {
    let (sender, receiver) = mpsc::sync_channel(1);
    let lines = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Capture(lines.clone()));
    tracing::subscriber::with_default(subscriber, || {
        report(&sender, "kept".to_owned());
        for payload in [
            "http://127.0.0.1:8000/?tkn=fictional-secret",
            "private page title",
            "private annotation body",
        ] {
            report(&sender, payload.to_owned());
        }
    });
    assert_eq!(receiver.try_iter().collect::<Vec<_>>(), ["kept"]);
    assert_eq!(
        lines.lock().unwrap().as_slice(),
        [
            "message=Dropped a browser event: queue full",
            "message=Dropped a browser event: queue full",
            "message=Dropped a browser event: queue full",
        ]
    );
    drop(receiver);
    report(&sender, "closed queue".to_owned());
}
