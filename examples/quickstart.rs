//! # Iroh Ping Quickstart Example
//!
//! This example demonstrates how to use iroh-ping to send ping requests between two endpoints.
//!
//! ## Usage
//!
//! First, start the receiver in one terminal:
//! ```sh
//! cargo run --example quickstart receiver
//! ```
//!
//! The receiver will print a ticket. Copy this ticket, then in another terminal run:
//! ```sh
//! cargo run --example quickstart sender <TICKET>
//! ```
//!
//! Pass `--flood` to follow the ping with a stream of bytes, printing the data
//! rate every second until Ctrl+C:
//! ```sh
//! cargo run --example quickstart sender --flood <TICKET>
//! ```
//!
//! Replace `<TICKET>` with the ticket printed by the receiver.

use std::{
    env,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use iroh::{Endpoint, endpoint::presets, protocol::Router};
use iroh_ping::Ping;
use iroh_tickets::{Ticket, endpoint::EndpointTicket};

async fn run_receiver() -> Result<()> {
    // Create an endpoint, it allows creating and accepting
    // connections in the iroh p2p world
    let preset = iroh_services::preset().build()?;

    // Wait for the endpoint to be accessible by others on the internet
    let endpoint = Endpoint::bind(preset.clone()).await?;
    endpoint.online().await;

    // continues reporting in the background.
    let client = preset
        .client_builder(&endpoint)
        .name("iroh-ping-quickstart")?
        .build()
        .await?;
    println!("registered with iroh-services, pushing endpoint metrics");
    // Then we initialize a struct that can accept ping requests over iroh connections
    let ping = Ping::new();

    // get the address of this endpoint to share with the sender
    let ticket = EndpointTicket::new(endpoint.addr());
    println!("{ticket}");

    // receiving ping requests
    let _router = Router::builder(endpoint)
        .accept(iroh_ping::ALPN, ping)
        .spawn();

    // Keep the receiver running until Ctrl+C
    tokio::signal::ctrl_c().await?;

    Ok(())
}

async fn run_sender(ticket: EndpointTicket, flood: bool) -> Result<()> {
    // create a send side & send a ping
    let preset = iroh_services::preset().build()?;

    // Wait for the endpoint to be accessible by others on the internet
    let endpoint = Endpoint::bind(preset.clone()).await?;

    endpoint.online().await;
    // continues reporting in the background.
    let client = preset
        .client_builder(&endpoint)
        .name("iroh-ping-quickstart-sender")?
        .build()
        .await?;
    println!("registered with iroh-services, pushing endpoint metrics");

    let send_pinger = Ping::new();
    let rtt = send_pinger
        .ping(&endpoint, ticket.endpoint_addr().clone())
        .await?;
    println!("ping took: {:?} to complete", rtt);

    // Then optionally push bytes to see what the connection can do, printing
    // the rate every second until Ctrl+C.
    if flood {
        println!("flooding, press Ctrl+C to stop");
        let metrics = send_pinger.metrics().clone();
        let flooding = send_pinger.flood(&endpoint, ticket.endpoint_addr().clone(), Duration::MAX);
        // Both of these have to be created once, outside the loop: a fresh
        // `ctrl_c` future each iteration would miss signals that arrive while
        // we're between registrations.
        let interrupt = tokio::signal::ctrl_c();
        tokio::pin!(flooding, interrupt);

        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.tick().await; // the first tick completes immediately
        let start = Instant::now();
        let mut last = 0;

        loop {
            tokio::select! {
                res = &mut flooding => { res?; break }
                _ = &mut interrupt => break,
                _ = ticker.tick() => {
                    let sent = metrics.bytes_sent.get();
                    println!("{:.2} MiB/s", mib(sent - last));
                    last = sent;
                }
            }
        }

        let sent = metrics.bytes_sent.get();
        println!(
            "sent {sent} bytes in {:?}: {:.2} MiB/s average",
            start.elapsed(),
            mib(sent) / start.elapsed().as_secs_f64()
        );
    }

    endpoint.close().await;
    Ok(())
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let mut args = env::args().skip(1);
    let role = args
        .next()
        .ok_or_else(|| anyhow!("expected 'receiver' or 'sender' as the first argument"))?;

    match role.as_str() {
        "receiver" => run_receiver().await,
        "sender" => {
            let args: Vec<String> = args.collect();
            let flood = args.iter().any(|arg| arg == "--flood");
            let ticket_str = args
                .iter()
                .find(|arg| !arg.starts_with("--"))
                .ok_or_else(|| anyhow!("expected ticket as the second argument"))?;
            let ticket = EndpointTicket::decode_string(ticket_str)
                .map_err(|e| anyhow!("failed to parse ticket: {}", e))?;

            run_sender(ticket, flood).await
        }
        _ => Err(anyhow!(
            "unknown role '{}'; use 'receiver' or 'sender'",
            role
        )),
    }
}
