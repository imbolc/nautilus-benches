use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use nautilus_common::{
    clients::DataClient,
    live::runner::{set_data_event_sender, set_system_event_sender},
    messages::{
        DataEvent,
        data::{
            SubscribeBookDeltas, SubscribeInstrumentClose, SubscribeInstrumentStatus,
            SubscribeQuotes, SubscribeTrades,
        },
    },
    providers::InstrumentProvider,
};
use nautilus_core::{UUID4, UnixNanos};
use nautilus_model::{data::Data, enums::BookType, identifiers::ClientId, instruments::Instrument};
use nautilus_network::{retry::RetryConfig, websocket::TransportBackend};
use nautilus_polymarket::{
    config::{PolymarketDataClientConfig, PolymarketInstrumentProviderConfig},
    data::PolymarketDataClient,
    http::{
        clob::PolymarketClobPublicClient, data_api::PolymarketDataApiHttpClient,
        gamma::PolymarketGammaHttpClient,
    },
    websocket::pool::PolymarketMarketConnectionPool,
};
use serde_json::json;

pub async fn run(count: usize) -> Result<()> {
    ensure!(
        count >= 2 && count <= 8000 && count.is_multiple_of(2),
        "instrument count must be even and between 2 and 8000"
    );
    let mut server = Command::new(std::env::current_exe()?)
        .arg("serve")
        .stdout(Stdio::piped())
        .spawn()?;
    let mut endpoint = String::new();
    let result = async {
        BufReader::new(server.stdout.take().context("fixture stdout")?).read_line(&mut endpoint)?;
        ensure!(!endpoint.is_empty(), "fixture server exited");
        sample(endpoint.trim(), count).await
    }
    .await;
    let _ = server.kill();
    server.wait()?;
    result
}

async fn sample(endpoint: &str, count: usize) -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
    set_data_event_sender(tx);
    let (system_tx, mut system_rx) = tokio::sync::mpsc::unbounded_channel();
    set_system_event_sender(system_tx);
    let config = PolymarketDataClientConfig {
        instrument_config: Some(PolymarketInstrumentProviderConfig {
            load_all: true,
            filters: Some(HashMap::from([
                ("active".into(), "true".into()),
                ("closed".into(), "false".into()),
                ("max_markets".into(), (count / 2).to_string()),
            ])),
            ..Default::default()
        }),
        subscribe_new_markets: true,
        resolve_poll_enabled: false,
        resolve_poll_interval_secs: 3600,
        book_snapshot_timeout_secs: 60,
        auto_load_missing_instruments: false,
        base_url_reference_prices: Some(format!(
            "{}/unused",
            endpoint.replacen("http://", "ws://", 1)
        )),
        ..Default::default()
    };
    let gamma = PolymarketGammaHttpClient::new(Some(endpoint.into()), 60, RetryConfig::default())?;
    let clob = PolymarketClobPublicClient::new(Some(endpoint.into()), 60)?;
    let data_api = PolymarketDataApiHttpClient::new(Some(endpoint.into()), 60)?;
    let ws = PolymarketMarketConnectionPool::new(
        Some(format!(
            "{}/ws/market",
            endpoint.replacen("http://", "ws://", 1)
        )),
        true,
        TransportBackend::default(),
        200,
    );
    let client_id = ClientId::from("SUBSCRIPTION-BENCH");
    let mut client = PolymarketDataClient::new(client_id, config, gamma, clob, data_api, ws)?;
    client.connect().await?;
    let mut instruments: Vec<_> = client.provider().store().get_all().values().collect();
    instruments.sort_by_key(|instrument| instrument.id());
    ensure!(
        instruments.len() == count,
        "expected {count} instruments, loaded {}",
        instruments.len()
    );
    let ids: Vec<_> = instruments
        .iter()
        .map(|instrument| instrument.id())
        .collect();
    let expected_ids: HashSet<_> = ids.iter().copied().collect();
    let expected_tokens: HashSet<_> = instruments
        .iter()
        .map(|instrument| instrument.raw_symbol().to_string())
        .collect();
    drop(instruments);
    while rx.try_recv().is_ok() {}
    while system_rx.try_recv().is_ok() {}

    // Hold the one runtime worker so every subscription future is still pending at the queue snapshot.
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    nautilus_common::live::get_runtime().spawn(async move {
        let _ = started_tx.send(());
        let _ = release_rx.recv();
    });
    started_rx.recv()?;
    let measurement = nautilus_bench::Measurement::begin()?;
    for id in &ids {
        client.subscribe_book_deltas(SubscribeBookDeltas::new(
            *id,
            BookType::L2_MBP,
            Some(client_id),
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
            true,
            None,
            None,
        ))?;
        client.subscribe_quotes(SubscribeQuotes::new(
            *id,
            Some(client_id),
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
            None,
        ))?;
        client.subscribe_trades(SubscribeTrades::new(
            *id,
            Some(client_id),
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
            None,
        ))?;
        client.subscribe_instrument_status(SubscribeInstrumentStatus::new(
            *id,
            Some(client_id),
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
            None,
        ))?;
        client.subscribe_instrument_close(SubscribeInstrumentClose::new(
            *id,
            Some(client_id),
            None,
            UUID4::new(),
            UnixNanos::default(),
            None,
            None,
        ))?;
    }
    let queued_heap = nautilus_bench::heap();
    let queued_rss = nautilus_bench::rss()?;
    release_tx.send(())?;

    let mut books = HashSet::new();
    let mut quotes = HashSet::new();
    tokio::time::timeout(Duration::from_secs(120), async {
        while books.len() < count || quotes.len() < count {
            match rx.recv().await.context("data channel closed")? {
                DataEvent::Data(Data::BookDeltas(deltas)) => {
                    books.insert(deltas.instrument_id);
                }
                DataEvent::Data(Data::Quote(quote)) => {
                    quotes.insert(quote.instrument_id);
                }
                _ => {}
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("initial book/quote drain timed out")??;
    ensure!(
        books == expected_ids && quotes == expected_ids,
        "initial data instrument set differs"
    );
    let snapshot_heap = nautilus_bench::heap();
    let snapshot_rss = nautilus_bench::rss()?;
    client.disconnect().await?;
    ensure!(client.is_disconnected(), "client did not disconnect");
    let metrics = measurement.finish()?;
    let actual: serde_json::Value = serde_json::from_slice(
        &Command::new("curl")
            .args(["-fsS", &format!("{endpoint}/stats")])
            .output()?
            .stdout,
    )?;
    let actual_tokens: HashSet<_> = actual["tokens"]
        .as_array()
        .context("fixture subscriptions")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    ensure!(
        actual_tokens == expected_tokens,
        "wire subscriptions differ from instrument tokens"
    );
    println!(
        "{}",
        json!({
            "benchmark": "polymarket-subscription-memory",
            "nautilus_revision": nautilus_bench::NAUTILUS_REVISION,
            "instruments": count, "subscription_families": 5, "queued_futures": count * 5,
            "runtime_workers": 1, "queued_heap": queued_heap, "queued_rss": queued_rss,
            "snapshot_heap": snapshot_heap, "snapshot_rss": snapshot_rss, "metrics": metrics,
            "verified": {"wire_tokens": actual_tokens.len(), "initial_books": books.len(), "initial_quotes": quotes.len(), "disconnected": true}
        })
    );
    Ok(())
}
