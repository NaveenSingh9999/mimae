use mimae_core::{Coordinator, EventLogOptions, serve};
use std::sync::{Arc, Mutex};

fn usage() -> ! {
    eprintln!("usage: mimae-daemon --port <port> --log <path>");
    std::process::exit(2);
}

fn main() {
    let mut port: Option<u16> = None;
    let mut log: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                port = args.next().and_then(|p| p.parse().ok());
            }
            "--log" => {
                log = args.next();
            }
            _ => usage(),
        }
    }
    let (port, log) = match (port, log) {
        (Some(port), Some(log)) => (port, log),
        _ => usage(),
    };
    let (coordinator, info) =
        Coordinator::open(&log, EventLogOptions::default()).unwrap_or_else(|e| {
            eprintln!("open log {log}: {e}");
            std::process::exit(1);
        });
    eprintln!(
        "mimae-daemon: {} events, high-water seq {}",
        info.events, info.high_water_seq
    );
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("bind 127.0.0.1:{port}: {e}");
        std::process::exit(1);
    });
    eprintln!("mimae-daemon: listening on 127.0.0.1:{port}");
    if let Err(e) = serve(listener, Arc::new(Mutex::new(coordinator))) {
        eprintln!("serve error: {e}");
        std::process::exit(1);
    }
}
