# iroh ping

A very simple iroh protocol for pinging a remote node. It's a high level example & easy starting point for new projects:

```rust
use anyhow::Result;
use iroh::{protocol::Router, Endpoint, Watcher};
use iroh_ping::Ping;

#[tokio::main]
async fn main() -> Result<()> {
    // Create an endpoint, it allows creating and accepting
    // connections in the iroh p2p world
    let recv_ep = Endpoint::builder().bind().await?;

    // Then we initialize a struct that can accept ping requests over iroh connections
    let ping = Ping::new();

    // receiving ping requests
    let recv_router = Router::builder(recv_ep)
        .accept(iroh_ping::ALPN, ping)
        .spawn();

    // get the address of this endpoint to share with the sender
    let addr = recv_router.endpoint().addr();

    // create a send side & send a ping
    let send_ep = Endpoint::builder().bind().await?;
    let send_pinger = Ping::new();
    let rtt = send_pinger.ping(&send_ep, addr).await?;

    println!("ping took: {:?} to complete", rtt);
    Ok(())
}

```

## This is not the "real" ping

Iroh has all sorts of internal ping-type messages, this is a high level demo of a protocol, and in no way necessary for iroh's normal operation.
