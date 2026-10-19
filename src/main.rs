use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use nautilus_common::{
    clients::DataClient, live::runner::set_data_event_sender, messages::DataEvent,
    providers::InstrumentProvider,
};
use nautilus_model::identifiers::ClientId;
use nautilus_network::{retry::RetryConfig, websocket::config::TransportBackend};
use nautilus_polymarket::{
    common::consts::WS_DEFAULT_SUBSCRIPTIONS,
    config::{PolymarketDataClientConfig, PolymarketInstrumentProviderConfig},
    data::PolymarketDataClient,
    http::{
        clob::PolymarketClobPublicClient, data_api::PolymarketDataApiHttpClient,
        gamma::PolymarketGammaHttpClient,
    },
    websocket::pool::PolymarketMarketConnectionPool,
};
use serde_json::{Value, json};
use tabled::{
    builder::Builder,
    grid::config::Offset,
    settings::{Alignment, Style, object::Rows, style::LineChar},
};
use tiny_http::{Header, Response, Server};
use url::Url;

const SOURCE: &str =
    "https://gamma-api.polymarket.com/markets/keyset?limit=100&active=true&closed=false";
const DATA_DIR: &str = "data";
const MARKET_COUNTS: [usize; 4] = [500, 1000, 2000, 4000];

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let data = Path::new(DATA_DIR);
    match args.as_slice() {
        [command] if command == "capture" => capture(data, MARKET_COUNTS[MARKET_COUNTS.len() - 1]),
        [command] if command == "serve" => serve(data),
        [command] if command == "run" => run(data).await,
        _ => bail!("invalid command"),
    }
}

async fn run(data: &Path) -> Result<()> {
    // Each sample starts with fresh Nautilus state.
    let Some(case) = std::env::var_os("NAUTILUS_BENCH_CASE") else {
        let index: Value = serde_json::from_reader(fs::File::open(data.join("index.json"))?)?;
        let available = index["markets"].as_u64().context("fixture market count")?;
        let required = MARKET_COUNTS[MARKET_COUNTS.len() - 1];
        if available < required as u64 {
            bail!(
                "fixture has {available} markets; {required} required. Move data/ aside and run capture"
            );
        }
        let mut table = Builder::default();
        table.push_record([
            "Markets",
            "Instruments",
            "CPU (s)",
            "CPU/instrument (µs)",
            "RAM/instrument (KiB)",
        ]);
        for (case, markets) in MARKET_COUNTS.iter().enumerate() {
            let output = Command::new(std::env::current_exe()?)
                .arg("run")
                .env("NAUTILUS_BENCH_CASE", case.to_string())
                .stderr(Stdio::inherit())
                .output()?;
            if !output.status.success() {
                bail!("benchmark for {markets} markets failed: {}", output.status);
            }
            let row = std::str::from_utf8(&output.stdout)?;
            table.push_record(row.trim().split(','));
        }
        let mut table = table.build();
        table
            .with(Style::markdown())
            .with(Alignment::right())
            .modify(Rows::one(1), LineChar::horizontal(':', Offset::End(0)));
        println!("{table}");
        return Ok(());
    };
    let case = case
        .to_str()
        .context("invalid benchmark case")?
        .parse::<usize>()?;
    let requested = *MARKET_COUNTS.get(case).context("invalid benchmark case")?;
    let mut server = Command::new(std::env::current_exe()?)
        .arg("serve")
        .stdout(Stdio::piped())
        .spawn()?;
    let mut endpoint = String::new();
    let result = async {
        BufReader::new(server.stdout.take().context("fixture server stdout")?)
            .read_line(&mut endpoint)?;
        if endpoint.is_empty() {
            bail!("fixture server exited without an endpoint");
        }
        tokio::task::LocalSet::new()
            .run_until(benchmark(endpoint.trim(), requested))
            .await
    }
    .await;
    let _ = server.kill();
    server.wait()?;
    result
}

fn capture(data: &Path, requested: usize) -> Result<()> {
    fs::create_dir(data)?;
    let captured_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut cursors = Vec::new();
    let mut cursor: Option<String> = None;
    let mut markets = 0;

    while markets < requested {
        let mut url = Url::parse(SOURCE)?;
        if let Some(cursor) = &cursor {
            url.query_pairs_mut().append_pair("after_cursor", cursor);
        }
        let file = data.join(format!("{:04}.json", cursors.len()));
        let status = Command::new("curl")
            .args(["-fsS", "--output"])
            .arg(&file)
            .arg(url.as_str())
            .status()?;
        if !status.success() {
            bail!("Gamma capture failed: {status}");
        }
        // Save the untouched response before inspecting it; replay serves these
        // exact bytes.
        let page: Value = serde_json::from_reader(fs::File::open(file)?)?;
        markets += page["markets"]
            .as_array()
            .context("Gamma markets array")?
            .len();
        cursors.push(cursor);
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        eprintln!("Captured {markets} markets");
        if cursor.is_none() {
            break;
        }
    }

    fs::write(
        data.join("index.json"),
        serde_json::to_vec_pretty(&json!({
            "source": SOURCE,
            "captured_at_unix": captured_at,
            "markets": markets,
            "cursors": cursors,
        }))?,
    )?;
    let hashes = Command::new("sha256sum")
        .current_dir(data)
        .arg("index.json")
        .args((0..cursors.len()).map(|index| format!("{index:04}.json")))
        .output()?;
    if !hashes.status.success() {
        bail!(
            "sha256sum failed: {}",
            String::from_utf8_lossy(&hashes.stderr)
        );
    }
    fs::write(data.join("SHA256SUMS"), hashes.stdout)?;
    Ok(())
}

fn serve(data: &Path) -> Result<()> {
    let index: Value = serde_json::from_reader(fs::File::open(data.join("index.json"))?)?;
    let cursors = index["cursors"].as_array().context("fixture cursors")?;
    let server = Server::http(("127.0.0.1", 0)).map_err(anyhow::Error::msg)?;
    println!("http://{}", server.server_addr());
    std::io::stdout().flush()?;

    for request in server.incoming_requests() {
        let url = Url::parse(&format!("http://localhost{}", request.url()))?;
        let cursor = url
            .query_pairs()
            .find(|(key, _)| key == "after_cursor")
            .map(|(_, value)| value.into_owned());
        let page = cursors
            .iter()
            .position(|value| value.as_str() == cursor.as_deref());
        let (status, body) = match (url.path(), page) {
            ("/markets/keyset", Some(page)) => {
                (200, fs::read(data.join(format!("{page:04}.json")))?)
            }
            _ => (404, b"fixture response not captured".to_vec()),
        };
        let response = Response::from_data(body)
            .with_status_code(status)
            .with_header("Content-Type: application/json".parse::<Header>().unwrap());
        request.respond(response)?;
    }
    Ok(())
}

async fn benchmark(endpoint: &str, requested: usize) -> Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
    set_data_event_sender(sender);
    let config = PolymarketDataClientConfig {
        instrument_config: Some(PolymarketInstrumentProviderConfig {
            load_all: true,
            filters: Some(HashMap::from([
                ("active".into(), "true".into()),
                ("closed".into(), "false".into()),
                ("max_markets".into(), requested.to_string()),
            ])),
            ..Default::default()
        }),
        resolve_poll_enabled: false,
        ..Default::default()
    };
    let gamma = PolymarketGammaHttpClient::new(Some(endpoint.into()), 60, RetryConfig::default())?;
    let clob = PolymarketClobPublicClient::new(Some(endpoint.into()), 60)?;
    let data_api = PolymarketDataApiHttpClient::new(Some(endpoint.into()), 60)?;
    let ws = PolymarketMarketConnectionPool::new(
        None,
        false,
        TransportBackend::default(),
        WS_DEFAULT_SUBSCRIPTIONS,
    );
    let mut client = PolymarketDataClient::new(
        ClientId::from("BOOTSTRAP-BENCH"),
        config,
        gamma,
        clob,
        data_api,
        ws,
    );
    let tick_output = Command::new("getconf").arg("CLK_TCK").output()?;
    let ticks: f64 = std::str::from_utf8(&tick_output.stdout)?.trim().parse()?;
    let cpu_start = cpu_ticks()?;
    client.connect().await?;
    let cpu = (cpu_ticks()? - cpu_start) as f64 / ticks;
    let status = fs::read_to_string("/proc/self/status")?;
    let peak_rss = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.split_whitespace().next())
        .context("Linux peak RSS")?
        .parse::<u64>()?;
    let instruments = client.provider().store().count();
    // Nautilus buffers instrument events until data clients finish connecting.
    let mut published = 0;
    while let Ok(event) = receiver.try_recv() {
        if matches!(event, DataEvent::Instrument(_)) {
            published += 1;
        }
    }
    client.disconnect().await?;
    if instruments == 0 || instruments != published {
        bail!("invalid instrument counts: stored={instruments}, published={published}");
    }
    let cpu_per_instrument = cpu * 1_000_000.0 / instruments as f64;
    let ram_per_instrument = peak_rss as f64 / instruments as f64;
    println!("{requested},{instruments},{cpu:.2},{cpu_per_instrument:.2},{ram_per_instrument:.2}");
    Ok(())
}

fn cpu_ticks() -> Result<u64> {
    let stat = fs::read_to_string("/proc/self/stat")?;
    let (_, fields) = stat.rsplit_once(") ").context("Linux process stat")?;
    // The tail begins at field 3; user and system CPU ticks are fields 14 and
    // 15.
    fields
        .split_whitespace()
        .skip(11)
        .take(2)
        .map(|value| value.parse::<u64>().map_err(Into::into))
        .sum()
}
