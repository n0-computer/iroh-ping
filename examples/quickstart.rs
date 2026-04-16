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
//! Replace `<TICKET>` with the ticket printed by the receiver.

use std::env;

use anyhow::{Result, anyhow};
use iroh::{Endpoint, endpoint::presets, protocol::Router};
use iroh_ping::Ping;
use iroh_tickets::{Ticket, endpoint::EndpointTicket};

async fn run_receiver() -> Result<()> {
    // Create an endpoint, it allows creating and accepting
    // connections in the iroh p2p world
    let endpoint = Endpoint::bind(presets::N0).await?;

    // Wait for the endpoint to be accessible by others on the internet
    endpoint.online().await;

    // Optionally push endpoint metrics to iroh-services if an API secret is
    // available. Keep the client bound for the lifetime of the receiver so it
    // continues reporting in the background.
    let _services_client = match env::var("IROH_SERVICES_API_SECRET") {
        Ok(_) => {
            let client = iroh_services::Client::builder(&endpoint)
                .api_secret_from_env()?
                .name("iroh-ping-quickstart")?
                .build()
                .await?;
            println!("registered with iroh-services, pushing endpoint metrics");
            Some(client)
        }
        Err(_) => {
            println!(
                "IROH_SERVICES_API_SECRET not set, skipping iroh-services setup. \
                 Get a free API key at https://services.iroh.computer to see endpoint metrics and debug connectivity issues."
            );
            None
        }
    };

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

async fn run_sender(ticket: EndpointTicket) -> Result<()> {
    // create a send side & send a ping
    let send_ep = Endpoint::bind(presets::N0).await?;
    let send_pinger = Ping::new();
    let rtt = send_pinger
        .ping(&send_ep, ticket.endpoint_addr().clone())
        .await?;
    println!("ping took: {:?} to complete", rtt);
    send_ep.close().await;
    Ok(())
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
            let ticket_str = args
                .next()
                .ok_or_else(|| anyhow!("expected ticket as the second argument"))?;
            let ticket = EndpointTicket::deserialize(&ticket_str)
                .map_err(|e| anyhow!("failed to parse ticket: {}", e))?;

            run_sender(ticket).await
        }
        _ => Err(anyhow!(
            "unknown role '{}'; use 'receiver' or 'sender'",
            role
        )),
    }
}
