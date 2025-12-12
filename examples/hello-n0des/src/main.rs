use anyhow::{anyhow, Result};
use iroh::{Endpoint, protocol::Router};
use iroh_ping::Ping;
use iroh_tickets::{Ticket, endpoint::EndpointTicket};
use std::env;

async fn run_receiver() -> Result<()> {
    // Create an endpoint, it allows creating and accepting
    // connections in the iroh p2p world
    let endpoint = Endpoint::builder().bind().await?;

    // bring the endpoint online before accepting connections
    endpoint.online().await;

    // Then we initialize a struct that can accept ping requests over iroh connections
    let ping = Ping::new();

    // get the address of this endpoint to share with the sender
    let ticket = EndpointTicket::new(endpoint.addr());
    println!("{ticket}");

    // receiving ping requests
    Router::builder(endpoint)
        .accept(iroh_ping::ALPN, ping)
        .spawn();

    // Keep the receiver running indefinitely
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }

}

async fn run_sender(ticket: EndpointTicket) -> Result<()> {
    // create a send side & send a ping
    let send_ep = Endpoint::builder().bind().await?;
    let send_pinger = Ping::new();
    let rtt = send_pinger.ping(&send_ep, ticket.endpoint_addr().clone()).await?;
    println!("ping took: {:?} to complete", rtt);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
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
        _ => Err(anyhow!("unknown role '{}'; use 'receiver' or 'sender'", role)),
    }
}