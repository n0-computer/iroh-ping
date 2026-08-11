use anyhow::Result;
use iroh::{Endpoint, endpoint::presets, protocol::Router};
use iroh_ping::Ping;

#[tokio::main]
async fn main() -> Result<()> {
    // create the receive side
    let preset = iroh_services::preset()
        .relays([
            "https://eft3wahhdmz0e2wxz.euc1.relay.iroh-svc.com/",
            "https://eft3wahhdmz0e2wxz.use1.relay.iroh-svc.com/",
            "https://eft3wahhdmz0e2wxz.usw1.relay.iroh-svc.com/",
            "https://eft3wahhdmz0e2wxz.aps1.relay.iroh-svc.com/",
        ])?
        .api_secret_from_str("servicesaaqjkypa7vnbreg2hrelm3akosv3m2hk572eupngzcf7ktj2cnyoib5ae2pmqcghaelsguu27vamswkodcxcgtolybqxgg6ruxw6mk2pvqaa")?
        .build()?;

    let recv_ep = Endpoint::bind(preset.clone()).await?;
    let recv_router = Router::builder(recv_ep.clone())
        .accept(iroh_ping::ALPN, Ping::new())
        .spawn();
    recv_ep.online().await;
    let addr = recv_router.endpoint().addr();

    let client = preset
        .clone()
        .client_builder(&recv_ep)
        .name("quickstart-example")?
        .build()
        .await?;

    // create a send side & send a ping
    let send_ep = Endpoint::bind(presets::N0).await?;
    let send_pinger = Ping::new();
    let rtt = send_pinger.ping(&send_ep, addr).await?;
    println!("ping took: {rtt:?} to complete");

    send_ep.close().await;
    recv_ep.close().await;

    Ok(())
}
