//! Runs the test broker on the loopback address until the process is stopped.
//!
//! Used by `tools/measure.ps1` to measure the application with Home Assistant on, and by
//! hand to try the application without a real broker. Never shipped.
//!
//! Arguments: `[--port N] [--user NAME --password SECRET]`. Prints `port N` once listening,
//! then a line `connected=C retained=R availability=A` whenever one of those changes (C:
//! clients connected now, R: retained messages, A: the last availability payload published
//! under `bettervolume/`, or `none`). The tool reads those lines.

use std::io::Write;
use std::time::Duration;

use bv_mqtt::broker::{Broker, BrokerConfig};

fn main() -> std::io::Result<()> {
    let mut config = BrokerConfig::default();
    let mut args = std::env::args().skip(1);
    let (mut user, mut password) = (None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => config.port = args.next().and_then(|p| p.parse().ok()).unwrap_or(0),
            "--user" => user = args.next(),
            "--password" => password = args.next(),
            _ => {}
        }
    }
    config.credentials = user.zip(password);
    let broker = Broker::start(config)?;
    let mut out = std::io::stdout();
    writeln!(out, "port {}", broker.port())?;
    out.flush()?;
    let mut last = String::new();
    loop {
        let topics = broker.retained_topics();
        let availability = topics
            .iter()
            .find(|t| t.starts_with("bettervolume/") && t.ends_with("/availability"))
            .and_then(|t| broker.retained(t))
            .map_or_else(|| "none".to_owned(), |p| String::from_utf8_lossy(&p).into_owned());
        let line = format!(
            "connected={} retained={} availability={availability}",
            broker.connected(),
            topics.len()
        );
        if line != last {
            writeln!(out, "{line}")?;
            out.flush()?;
            last = line;
        }
        // A test tool: looking every 50 ms is fine here (never in the application).
        std::thread::sleep(Duration::from_millis(50));
    }
}
