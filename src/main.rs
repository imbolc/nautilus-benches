mod fixture;

use std::{collections::HashMap, process::ExitCode, time::Duration};

use anyhow::{Context, Result, ensure};
use nautilus_common::{
    clients::DataClient,
    live::runner::{set_data_event_sender, set_system_event_sender},
    messages::DataEvent,
    msgbus,
};
use nautilus_core::{
    UUID4, UnixNanos,
    datetime::{iso8601_to_unix_nanos, unix_nanos_to_iso8601},
};
use nautilus_model::{
    data::Data,
    enums::{OrderSide, PositionSide},
    events::{PositionEvent, PositionOpened},
    identifiers::{AccountId, ClientId, ClientOrderId, PositionId, StrategyId, TraderId},
    instruments::{Instrument, InstrumentAny},
    types::{Price, Quantity},
};
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
const OBSERVATION_SECS: u64 = 65;

fn main() -> ExitCode {
    let result = nautilus_common::live::get_runtime()
        .block_on(tokio::task::LocalSet::new().run_until(run()));
    match result {
        Ok(true) => {
            println!("FRESH");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("STALE");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::from(2)
        }
    }
}

async fn run() -> Result<bool> {
    let (venue, endpoint, server) = fixture::serve().await?;
    let result = reproduce(&venue, &endpoint).await;
    server.abort();
    result
}

async fn reproduce(venue: &fixture::Venue, endpoint: &str) -> Result<bool> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
    set_data_event_sender(tx);
    let (system_tx, _system_rx) = tokio::sync::mpsc::unbounded_channel();
    set_system_event_sender(system_tx);
    let config = PolymarketDataClientConfig {
        instrument_config: Some(PolymarketInstrumentProviderConfig {
            load_all: true,
            filters: Some(HashMap::from([
                ("active".into(), "true".into()),
                ("closed".into(), "false".into()),
                ("end_date_max".into(), fixture::DISCOVERY_END.into()),
            ])),
            ..Default::default()
        }),
        update_instruments_interval_mins: Some(1),
        subscribe_new_markets: false,
        resolve_poll_enabled: false,
        resolve_poll_interval_secs: 3600,
        auto_load_missing_instruments: false,
        base_url_reference_prices: Some(endpoint.replacen("http://", "ws://", 1)),
        ..Default::default()
    };
    let gamma = PolymarketGammaHttpClient::new(Some(endpoint.into()), 5, RetryConfig::default())?;
    let clob = PolymarketClobPublicClient::new(Some(endpoint.into()), 5)?;
    let data_api = PolymarketDataApiHttpClient::new(Some(endpoint.into()), 5)?;
    let ws = PolymarketMarketConnectionPool::new(
        Some(format!(
            "{}/ws/market",
            endpoint.replacen("http://", "ws://", 1)
        )),
        false,
        TransportBackend::default(),
        200,
    );
    let mut client = PolymarketDataClient::new(
        ClientId::from("RETAINED-REFRESH"),
        config,
        gamma,
        clob,
        data_api,
        ws,
    )?;
    tokio::time::timeout(Duration::from_secs(15), client.connect())
        .await
        .context("client connect timed out")??;

    let result = async {
        let mut initial = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let DataEvent::Data(Data::Instrument(instrument)) = event {
                initial.push(*instrument);
            }
        }
        ensure!(
            initial.len() == 2,
            "expected two bootstrap outcome definitions"
        );
        let instrument = initial
            .iter()
            .find(|instrument| instrument.raw_symbol().as_str() == "1")
            .context("Yes outcome not bootstrapped")?;
        let id = instrument.id();
        let original_expiration = instrument
            .expiration_ns()
            .context("missing initial expiration")?;

        msgbus::publish_position_event(
            "events.position.RETAINED-REFRESH".into(),
            &open_position(instrument),
        );
        venue.revise();

        // The public refresh interval is expressed in minutes. Allow one cycle plus delivery slack.
        tokio::time::sleep(Duration::from_secs(OBSERVATION_SECS)).await;
        let mut observed_expiration = original_expiration;
        while let Ok(event) = rx.try_recv() {
            if let DataEvent::Data(Data::Instrument(instrument)) = event
                && instrument.id() == id
            {
                observed_expiration = instrument
                    .expiration_ns()
                    .context("missing refreshed expiration")?;
            }
        }
        ensure!(
            venue.discovery_refreshed(),
            "no completed discovery request excluded the revised market"
        );
        let expected = iso8601_to_unix_nanos(fixture::REVISED_END)?;
        ensure!(
            expected != original_expiration,
            "fixture closing time did not change"
        );
        ensure!(
            observed_expiration == original_expiration || observed_expiration == expected,
            "unexpected emitted closing time"
        );
        println!("Venue:    {}", unix_nanos_to_iso8601(expected));
        println!("Nautilus: {}", unix_nanos_to_iso8601(observed_expiration));
        Ok(observed_expiration == expected)
    }
    .await;

    tokio::time::timeout(Duration::from_secs(15), client.disconnect())
        .await
        .context("client disconnect timed out")??;
    ensure!(client.is_disconnected(), "client did not disconnect");
    result
}

fn open_position(instrument: &InstrumentAny) -> PositionEvent {
    PositionEvent::PositionOpened(PositionOpened {
        trader_id: TraderId::from("TRADER-001"),
        strategy_id: StrategyId::from("REPRO-001"),
        instrument_id: instrument.id(),
        position_id: PositionId::from("P-RETAINED"),
        account_id: AccountId::from("POLYMARKET-001"),
        opening_order_id: ClientOrderId::from("O-RETAINED"),
        entry: OrderSide::Buy,
        side: PositionSide::Long,
        signed_qty: 1.into(),
        quantity: Quantity::from("1"),
        last_qty: Quantity::from("1"),
        last_px: Price::from("0.500"),
        currency: instrument.quote_currency(),
        avg_px_open: "0.500".parse().unwrap(),
        realized_pnl: None,
        event_id: UUID4::new(),
        ts_event: UnixNanos::from(1),
        ts_init: UnixNanos::from(1),
    })
}
