use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    routing::get,
};
use serde_json::{Value, json};

#[derive(Clone)]
struct Venue {
    pages: Arc<HashMap<Option<String>, Value>>,
    tokens: Arc<HashMap<String, String>>,
    subscriptions: Arc<Mutex<HashSet<String>>>,
}

pub async fn serve() -> Result<()> {
    let root = std::path::PathBuf::from("data");
    let index: Value = serde_json::from_slice(&std::fs::read(root.join("index.json"))?)?;
    let mut pages = HashMap::new();
    let mut tokens = HashMap::new();
    for (page, cursor) in index["cursors"]
        .as_array()
        .context("fixture cursors")?
        .iter()
        .enumerate()
    {
        let mut body: Value =
            serde_json::from_slice(&std::fs::read(root.join(format!("{page:04}.json")))?)?;
        for market in body["markets"].as_array_mut().context("fixture markets")? {
            // Fixed expiry keeps historical inputs independent of the execution date
            market["endDate"] = json!("2099-01-01T00:00:00Z");
            market["endDateIso"] = json!("2099-01-01");
            let ids: Vec<String> = serde_json::from_str(
                market["clobTokenIds"]
                    .as_str()
                    .context("fixture token IDs")?,
            )?;
            for token in ids {
                tokens.insert(
                    token,
                    market["conditionId"]
                        .as_str()
                        .context("condition ID")?
                        .to_owned(),
                );
            }
        }
        pages.insert(cursor.as_str().map(str::to_owned), body);
    }
    let state = Venue {
        pages: Arc::new(pages),
        tokens: Arc::new(tokens),
        subscriptions: Arc::default(),
    };
    let app = Router::new()
        .route("/markets/keyset", get(markets))
        .route("/ws/market", get(upgrade))
        .route("/stats", get(stats))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    println!("http://{}", listener.local_addr()?);
    io::stdout().flush()?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn markets(
    State(venue): State<Venue>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    let cursor = query.get("after_cursor").cloned();
    Json(
        venue
            .pages
            .get(&cursor)
            .cloned()
            .unwrap_or_else(|| json!({"markets": []})),
    )
}

async fn stats(State(venue): State<Venue>) -> Json<Value> {
    let mut tokens: Vec<_> = venue
        .subscriptions
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    tokens.sort();
    Json(json!({"tokens": tokens}))
}

async fn upgrade(ws: WebSocketUpgrade, State(venue): State<Venue>) -> axum::response::Response {
    ws.on_upgrade(move |socket| stream(socket, venue))
}

async fn stream(mut socket: WebSocket, venue: Venue) {
    while let Some(Ok(message)) = socket.recv().await {
        match message {
            Message::Text(text) if text == "PING" => {
                if socket.send(Message::Text("PONG".into())).await.is_err() {
                    break;
                }
            }
            Message::Text(text) => {
                let Ok(body) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let Some(tokens) = body["assets_ids"].as_array() else {
                    continue;
                };
                for token in tokens.iter().filter_map(Value::as_str) {
                    if body["operation"] == "unsubscribe" {
                        venue.subscriptions.lock().unwrap().remove(token);
                        continue;
                    }
                    venue.subscriptions.lock().unwrap().insert(token.to_owned());
                    let Some(condition) = venue.tokens.get(token) else {
                        continue;
                    };
                    let snapshot = format!(
                        "{{\"event_type\":\"book\",\"market\":\"{condition}\",\"asset_id\":\"{token}\",\"bids\":[{{\"price\":\"0.4\",\"size\":\"10\"}}],\"asks\":[{{\"price\":\"0.6\",\"size\":\"20\"}}],\"timestamp\":\"1792394885000\",\"version\":\"v1\"}}"
                    );
                    if socket.send(Message::Text(snapshot.into())).await.is_err() {
                        return;
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(bytes) => {
                if socket.send(Message::Pong(bytes)).await.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
}
