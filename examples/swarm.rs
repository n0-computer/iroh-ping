//! Run independent dev/prod endpoints: cargo run --example swarm -- swarm.toml
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use futures_lite::StreamExt;
use iroh::{Endpoint, SecretKey, address_lookup::memory::MemoryLookup, protocol::Router};
use iroh_gossip::{
    TopicId,
    api::{Event, GossipTopic},
    net::{GOSSIP_ALPN, Gossip},
};
use iroh_ping::Ping;
use iroh_services::{CLIENT_HOST_ALPN, Client, ClientHost, caps::Caps};
use serde::Deserialize;
use tokio::{
    task::JoinSet,
    time::{Instant, sleep, timeout},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    api_key: Option<String>,
    #[serde(default = "default_api_key_env")]
    api_key_env: String,
    #[serde(default = "default_identity_dir")]
    identity_dir: PathBuf,
    relays: Vec<String>,
    metrics_interval_secs: u64,
    duration_secs: u64,
    swarms: Vec<Swarm>,
}

fn default_api_key_env() -> String {
    "IROH_SERVICES_API_SECRET".into()
}

fn default_identity_dir() -> PathBuf {
    ".swarm-identities".into()
}

fn device_key(directory: &Path, swarm: &str, index: usize) -> Result<SecretKey> {
    // Encode the internal swarm name so arbitrary names cannot escape this directory.
    let name: String = swarm.bytes().map(|byte| format!("{byte:02x}")).collect();
    let path = directory.join(format!("{name}-{:03}.key", index + 1));
    let read_key = || -> Result<SecretKey> {
        let bytes =
            fs::read(&path).with_context(|| format!("reading identity {}", path.display()))?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
            anyhow::anyhow!(
                "identity {} must contain exactly 32 bytes; refusing to replace it",
                path.display()
            )
        })?;
        Ok(SecretKey::from_bytes(&bytes))
    };
    match fs::metadata(&path) {
        Ok(_) => return read_key(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("checking identity {}", path.display()));
        }
    }
    fs::create_dir_all(directory)
        .with_context(|| format!("creating identity directory {}", directory.display()))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return read_key(),
        Err(error) => {
            return Err(error).with_context(|| format!("creating identity {}", path.display()));
        }
    };
    let key = SecretKey::generate();
    file.write_all(&key.to_bytes())
        .with_context(|| format!("saving identity {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing identity {}", path.display()))?;
    Ok(key)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Swarm {
    name: String,
    group: String,
    nodes: usize,
    flood_secs: u64,
    pause_secs: u64,
    attributes: BTreeMap<String, String>,
}

impl Config {
    fn parse(text: &str) -> Result<Self> {
        // Do not echo TOML parse errors: their source excerpt may include an API key.
        let config: Self = toml::from_str(text)
            .map_err(|_| anyhow::anyhow!("invalid swarm TOML; check field names and types"))?;
        ensure!(!config.swarms.is_empty(), "configure at least one swarm");
        ensure!(
            !config.identity_dir.as_os_str().is_empty(),
            "identity_dir must not be empty"
        );
        ensure!(
            config.metrics_interval_secs > 0,
            "metrics_interval_secs must be positive"
        );
        ensure!(
            config.metrics_interval_secs <= 86400,
            "metrics interval must be at most one day"
        );
        ensure!(
            config.duration_secs <= 604800,
            "duration must be at most seven days (or 0)"
        );
        let mut names = BTreeSet::new();
        for swarm in &config.swarms {
            ensure!(
                !swarm.name.is_empty() && swarm.name.len() <= 64,
                "swarm name must be 1–64 bytes"
            );
            ensure!(
                names.insert(&swarm.name),
                "duplicate swarm name: {}",
                swarm.name
            );
            ensure!(
                (2..=128).contains(&swarm.group.len()),
                "group must be 2–128 bytes"
            );
            ensure!(
                (2..=1000).contains(&swarm.nodes),
                "{}: nodes must be 2–1000",
                swarm.name
            );
            ensure!(
                (1..=3600).contains(&swarm.flood_secs),
                "flood_secs must be 1–3600"
            );
            ensure!(
                swarm.pause_secs <= 86400,
                "pause_secs must be at most one day"
            );
            ensure!(
                swarm.attributes.len() <= 128,
                "at most 128 attributes allowed"
            );
            for (key, value) in &swarm.attributes {
                ensure!(
                    (2..=128).contains(&key.len()) && value.len() <= 128,
                    "attribute keys must be 2–128 bytes and values at most 128 bytes"
                );
            }
        }
        for relay in &config.relays {
            let _: iroh::RelayUrl = relay.parse().context("invalid relay URL")?;
        }
        Ok(config)
    }
}

struct Device {
    name: String,
    swarm: usize,
    router: Router,
    ping: Ping,
    client: Client,
    gossip: Gossip,
    lookup: MemoryLookup,
}

async fn start(config: &Config, devices: &mut Vec<Device>) -> Result<()> {
    let mut diagnostics: JoinSet<Result<()>> = JoinSet::new();
    let secret = match &config.api_key {
        Some(key) => key.clone(),
        None => env::var(&config.api_key_env)
            .with_context(|| format!("set {} or api_key in the config", config.api_key_env))?,
    };
    let mut base = iroh_services::preset()
        .api_secret_from_str(&secret)
        .map_err(|_| anyhow::anyhow!("invalid project API key"))?;
    if !config.relays.is_empty() {
        base = base.relays(&config.relays)?;
    }
    for (swarm_index, swarm) in config.swarms.iter().enumerate() {
        for index in 0..swarm.nodes {
            // Supply the persisted key before the preset scopes the relay token to it.
            let key = device_key(&config.identity_dir, &swarm.name, index)?;
            let preset = base.clone().secret_key(key).build()?;
            if devices.is_empty() {
                println!("services endpoint: {}", preset.api_secret().addr().id);
            }
            let lookup = MemoryLookup::new();
            let endpoint = Endpoint::builder(preset.clone())
                .address_lookup(lookup.clone())
                .bind()
                .await?;
            let gossip = Gossip::builder().spawn(endpoint.clone());
            let id = endpoint.id().to_string();
            let name = format!("{}-{:03}-{}", swarm.name, index + 1, &id[..8]);
            let ping = Ping::new();
            let client = preset
                .client_builder(&endpoint)
                .name(&name)?
                .group(&swarm.group)?
                .attributes(swarm.attributes.clone())?
                .register_metrics_group(ping.metrics().clone())
                .register_metrics_group(gossip.metrics().clone())
                .metrics_interval(Duration::from_secs(config.metrics_interval_secs))
                .build()
                .await?;
            let host = ClientHost::new(&endpoint);
            let router = Router::builder(endpoint)
                .accept(iroh_ping::ALPN, ping.clone())
                .accept(GOSSIP_ALPN, gossip.clone())
                .accept(CLIENT_HOST_ALPN, host)
                .spawn();
            devices.push(Device {
                name: name.clone(),
                swarm: swarm_index,
                router,
                ping,
                client,
                gossip,
                lookup,
            });
            let device = devices.last().unwrap();
            timeout(Duration::from_secs(30), async {
                device.router.endpoint().online().await;
                device.client.ping().await.map_err(authentication_error)?;
                // Explicit calls surface metadata errors that the builder only logs.
                device
                    .client
                    .set_name(&name)
                    .await
                    .context("setting device name")?;
                device
                    .client
                    .set_group(&swarm.group)
                    .await
                    .context("setting device group")?;
                device
                    .client
                    .set_attributes(swarm.attributes.clone())
                    .await
                    .context("setting device attributes")?;
                anyhow::Ok(())
            })
            .await
            .with_context(|| format!("{name}: startup timed out"))?
            .with_context(|| format!("{name}: services registration failed"))?;
            println!("ready {name} group={} endpoint={id}", swarm.group);
            let client = device.client.clone();
            let remote_id = preset.api_secret().addr().id;
            diagnostics.spawn(async move {
                timeout(Duration::from_secs(60), async {
                    client
                        .grant_capability(remote_id, Caps::net_diagnostics_get_any())
                        .await
                        .context(
                            "granting the project services endpoint network diagnostics access",
                        )?;
                    println!("{name}: running startup network diagnostics");
                    client
                        .net_diagnostics(true)
                        .await
                        .context("uploading startup network diagnostics")?;
                    println!("{name}: network diagnostics uploaded; remote diagnostics enabled");
                    anyhow::Ok(())
                })
                .await
                .with_context(|| format!("{name}: startup network diagnostics timed out"))?
                .with_context(|| format!("{name}: enabling startup network diagnostics"))
            });
        }
    }
    while let Some(result) = diagnostics.join_next().await {
        result.context("network diagnostics task failed")??;
    }
    Ok(())
}

fn authentication_error(error: iroh_services::Error) -> anyhow::Error {
    let error = anyhow::Error::new(error);
    let missing_reply = error
        .chain()
        .any(|cause| cause.to_string().contains("Oneshot recv error"));
    if missing_reply {
        error.context("services authentication returned no reply; check the backend authentication log for this endpoint and confirm the API key belongs to the current project")
    } else {
        error.context("checking services authentication/connectivity")
    }
}

// Only the bootstrap node's address is supplied out of band. Gossip introduces
// the other members and maintains each topic's live neighbor set.
async fn join_swarm(members: &[(&Endpoint, &Gossip, &MemoryLookup)]) -> Result<Vec<GossipTopic>> {
    let seed = members[0].0.addr();
    // Each swarm keeps a stable topic derived from its persisted seed identity.
    let topic = TopicId::from(*seed.id.as_bytes());
    let mut topics = Vec::new();
    for (endpoint, gossip, lookup) in members {
        let bootstrap = if endpoint.id() == seed.id {
            Vec::new()
        } else {
            lookup.add_endpoint_info(seed.clone());
            vec![seed.id]
        };
        topics.push(gossip.subscribe(topic, bootstrap).await?);
    }
    // Subscribe everyone before waiting: the seed needs another node to join.
    timeout(Duration::from_secs(30), async {
        for topic in &mut topics {
            topic.joined().await?;
        }
        anyhow::Ok(())
    })
    .await
    .context("gossip group join timed out")??;
    Ok(topics)
}

async fn traffic(config: &Config, devices: &[Device]) -> Result<()> {
    let mut workers: JoinSet<Result<()>> = JoinSet::new();
    for (swarm_index, swarm) in config.swarms.iter().enumerate() {
        let members: Vec<_> = devices.iter().filter(|d| d.swarm == swarm_index).collect();
        let handles: Vec<_> = members
            .iter()
            .map(|d| (d.router.endpoint(), &d.gossip, &d.lookup))
            .collect();
        let topics = join_swarm(&handles)
            .await
            .with_context(|| format!("joining {}", swarm.name))?;
        for (device, topic) in members.into_iter().zip(topics) {
            let endpoint = device.router.endpoint().clone();
            let ping = device.ping.clone();
            let name = device.name.clone();
            let flood = Duration::from_secs(swarm.flood_secs);
            let pause = Duration::from_secs(swarm.pause_secs);
            workers.spawn(async move {
                let (sender, mut receiver) = topic.split();
                let mut sequence = 0u64;
                loop {
                    let mut peers: Vec<_> = receiver.neighbors().collect();
                    peers.sort();
                    let peer = if peers.is_empty() { None } else { Some(peers[sequence as usize % peers.len()]) };
                    let current_sequence = sequence;
                    sequence = sequence.wrapping_add(1);
                    let round = async {
                        if let Some(peer) = peer {
                            // Include a sequence so gossip does not deduplicate repeated payloads.
                            let message = format!("{name}:{current_sequence}:{}", "x".repeat(1024));
                            sender.broadcast(message.into_bytes().into()).await?;
                            println!("traffic {name} -> {peer} (gossip neighbor)");
                            let transfer = async {
                                ping.ping(&endpoint, peer.into()).await?;
                                ping.flood(&endpoint, peer.into(), flood).await
                            };
                            match timeout(flood + Duration::from_secs(30), transfer).await {
                                Ok(Ok(_)) => {},
                                Ok(Err(error)) => eprintln!("{name}: transfer failed: {error}"),
                                Err(_) => eprintln!("{name}: transfer timed out"),
                            }
                        }
                        sleep(pause.max(Duration::from_millis(100))).await;
                        anyhow::Ok(())
                    };
                    tokio::pin!(round);
                    // Keep draining gossip events even while a flood is active.
                    loop {
                        tokio::select! {
                            result = &mut round => { result?; break; },
                            event = receiver.next() => match event.context("gossip event stream closed")?? {
                                Event::NeighborUp(peer) => println!("{name}: neighbor up {peer}"),
                                Event::NeighborDown(peer) => println!("{name}: neighbor down {peer}"),
                                Event::Received(_) => {},
                                Event::Lagged => eprintln!("{name}: gossip receiver lagged"),
                            }
                        }
                    }
                }
            });
        }
    }
    let started = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.tick().await;
    let deadline = async {
        if config.duration_secs == 0 {
            std::future::pending::<()>().await;
        }
        sleep(Duration::from_secs(config.duration_secs)).await;
    };
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            result = workers.join_next() => anyhow::bail!("traffic worker stopped unexpectedly: {result:?}"),
            _ = ticker.tick() => {
                for (index, swarm) in config.swarms.iter().enumerate() {
                    let (sent, received) = devices.iter().filter(|d| d.swarm == index)
                        .fold((0, 0), |(tx, rx), d| (tx + d.ping.metrics().bytes_sent.get(), rx + d.ping.metrics().bytes_recv.get()));
                    println!("{}: sent {:.1} MiB, received {:.1} MiB ({:.0}s)", swarm.name,
                        sent as f64 / 1048576.0, received as f64 / 1048576.0, started.elapsed().as_secs_f64());
                }
            }
        }
    }
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut path = None;
    let mut check = false;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --example swarm -- [swarm.toml] [--check]\n--check validates configuration without starting endpoints or requiring an API key."
                );
                return Ok(());
            }
            "--check" => check = true,
            _ => {
                ensure!(
                    !arg.starts_with('-') && path.is_none(),
                    "expected one config path and optional --check"
                );
                path = Some(arg);
            }
        }
    }
    let path = path.unwrap_or_else(|| "swarm.toml".into());
    let mut config =
        Config::parse(&std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?)?;
    if config.identity_dir.is_relative() {
        config.identity_dir = Path::new(&path)
            .parent()
            .unwrap_or(Path::new("."))
            .join(&config.identity_dir);
    }
    for swarm in &config.swarms {
        println!(
            "{}: {} devices, group={}, attributes={:?}",
            swarm.name, swarm.nodes, swarm.group, swarm.attributes
        );
    }
    if check {
        return Ok(());
    }
    let mut devices = Vec::new();
    let result = tokio::select! {
        result = async {
            start(&config, &mut devices).await?;
            println!("All devices ready. Press Ctrl+C to stop.");
            traffic(&config, &devices).await
        } => result,
        signal = tokio::signal::ctrl_c() => signal.context("listening for Ctrl+C"),
    };
    println!("Stopping devices and flushing metrics...");
    let mut shutdown = JoinSet::new();
    for device in devices {
        shutdown.spawn(async move {
            device.client.shutdown().await;
            device.router.shutdown().await
        });
    }
    while let Some(result) = shutdown.join_next().await {
        match result {
            Ok(Ok(())) => {}
            other => eprintln!("shutdown error: {other:?}"),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_and_topology() {
        let config = Config::parse(include_str!("../swarm.toml")).unwrap();
        assert_eq!(config.swarms.len(), 3);
        assert_eq!(
            config
                .swarms
                .iter()
                .map(|s| (s.group.as_str(), s.nodes))
                .collect::<Vec<_>>(),
            vec![("dev", 3), ("prod", 2), ("prod", 3)]
        );
        assert_ne!(
            config.swarms[0].attributes["app_version"],
            config.swarms[1].attributes["app_version"]
        );
        assert_ne!(
            config.swarms[1].attributes["app_version"],
            config.swarms[2].attributes["app_version"]
        );
    }

    #[test]
    fn identities_survive_restarts_and_stay_distinct() -> Result<()> {
        let directory = env::temp_dir().join(format!(
            "swarm-identities-{}",
            SecretKey::generate().public()
        ));
        let first = device_key(&directory, "prod-a", 0)?;
        let restarted = device_key(&directory, "prod-a", 0)?;
        let next = device_key(&directory, "prod-a", 1)?;
        let other = device_key(&directory, "prod-b", 0)?;
        assert_eq!(first.public(), restarted.public());
        assert_ne!(first.public(), next.public());
        assert_ne!(first.public(), other.public());
        // Corrupt state fails rather than silently changing a dashboard identity.
        let path = directory.join("70726f642d61-001.key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        }
        fs::write(&path, b"bad key")?;
        assert!(device_key(&directory, "prod-a", 0).is_err());
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn gossip_groups_exchange_data_in_isolation() -> Result<()> {
        use iroh::endpoint::presets;
        let mut nodes = Vec::new();
        for _ in 0..8 {
            let lookup = MemoryLookup::new();
            let endpoint = Endpoint::builder(presets::Minimal)
                .bind_addr("127.0.0.1:0")?
                .address_lookup(lookup.clone())
                .bind()
                .await?;
            let gossip = Gossip::builder().spawn(endpoint.clone());
            let ping = Ping::new();
            let router = Router::builder(endpoint)
                .accept(GOSSIP_ALPN, gossip.clone())
                .accept(iroh_ping::ALPN, ping.clone())
                .spawn();
            nodes.push((router, gossip, lookup, ping));
        }
        let mut subscriptions = Vec::new();
        for (group, range) in [(0, 0..3), (1, 3..5), (2, 5..8)] {
            let handles: Vec<_> = nodes[range.clone()]
                .iter()
                .map(|(router, gossip, lookup, _)| (router.endpoint(), gossip, lookup))
                .collect();
            let allowed: BTreeSet<_> = handles.iter().map(|(ep, _, _)| ep.id()).collect();
            for (index, topic) in range.zip(join_swarm(&handles).await?) {
                assert!(topic.neighbors().all(|id| allowed.contains(&id)));
                let peer = topic
                    .neighbors()
                    .next()
                    .context("missing gossip neighbor")?;
                let (sender, receiver) = topic.split();
                subscriptions.push((group, index, peer, allowed.clone(), sender, receiver));
            }
        }
        let mut receivers = JoinSet::new();
        let mut senders = Vec::new();
        for (group, index, peer, allowed, sender, mut receiver) in subscriptions {
            let (router, _, _, ping) = &nodes[index];
            // Data still flows over the ping/flood protocol to gossip-discovered peers.
            let (bytes, _) = timeout(
                Duration::from_secs(5),
                ping.flood(router.endpoint(), peer.into(), Duration::from_millis(20)),
            )
            .await??;
            assert!(bytes > 0);
            senders.push((group, index, sender));
            receivers.spawn(async move {
                let mut received = BTreeSet::new();
                while received.len() < allowed.len() - 1 {
                    match receiver.next().await.context("closed gossip stream")?? {
                        Event::Received(message) => {
                            assert_eq!(message.content[0], group);
                            assert_ne!(message.content[1], index as u8);
                            received.insert(message.content[1]);
                        }
                        Event::NeighborUp(id) => assert!(allowed.contains(&id)),
                        _ => {}
                    }
                }
                // Return the handle to keep every subscription alive until all finish.
                anyhow::Ok(receiver)
            });
        }
        for (group, index, sender) in &senders {
            sender.broadcast(vec![*group, *index as u8].into()).await?;
        }
        let mut held_receivers = Vec::new();
        timeout(Duration::from_secs(15), async {
            while let Some(result) = receivers.join_next().await {
                held_receivers.push(result??);
            }
            anyhow::Ok(())
        })
        .await
        .context("gossip messages did not reach all group members")??;
        drop(held_receivers);
        drop(senders);
        for (router, _, _, _) in nodes {
            router.shutdown().await?;
        }
        Ok(())
    }

    #[test]
    fn inline_api_key_does_not_require_env_setting() {
        let sample = include_str!("../swarm.toml");
        let mut text = String::from("api_key = \"test-key\"\n");
        for line in sample.lines() {
            if !line.starts_with("api_key =") && !line.starts_with("api_key_env =") {
                text.push_str(line);
                text.push('\n');
            }
        }
        let config = Config::parse(&text).unwrap();
        assert_eq!(config.api_key.as_deref(), Some("test-key"));
        assert_eq!(config.api_key_env, "IROH_SERVICES_API_SECRET");
    }

    #[test]
    fn rejects_bad_config_without_leaking_secrets() {
        let sample = include_str!("../swarm.toml");
        let interval = format!(
            "metrics_interval_secs = {}",
            Config::parse(sample).unwrap().metrics_interval_secs
        );
        for (from, to) in [
            ("nodes = 3", "nodes = 1"),
            ("flood_secs = 1", "flood_secs = 0"),
            (interval.as_str(), "metrics_interval_secs = 0"),
            (r#"name = "prod-a""#, r#"name = "dev""#),
        ] {
            assert!(Config::parse(&sample.replace(from, to)).is_err());
        }
        let error = Config::parse(
            r#"api_key = "super-secret"
bad = ["#,
        )
        .err()
        .unwrap();
        assert!(!error.to_string().contains("super-secret"));
    }
}
