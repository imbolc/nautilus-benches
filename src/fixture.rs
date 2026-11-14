use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde_json::{Value, json};

pub const DISCOVERY_END: &str = "2099-06-01T00:00:00Z";
pub const REVISED_END: &str = "2099-12-01T00:00:00Z";

#[derive(Clone)]
pub struct Venue {
    market: Value,
    revised: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Venue {
    pub fn revise(&self) {
        self.revised.store(true, Ordering::SeqCst);
    }

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

pub async fn serve() -> Result<(Venue, String, tokio::task::JoinHandle<std::io::Result<()>>)> {
    let venue = Venue {
        market: serde_json::from_str(include_str!("../data/market.json"))?,
        revised: Arc::default(),
        requests: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let app = Router::new()
        .route("/markets/keyset", get(markets))
        .with_state(venue.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((venue, endpoint, server))
}

async fn markets(
    State(venue): State<Venue>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    let revised = venue.revised.load(Ordering::SeqCst);
    let mut market = venue.market.clone();
    if revised {
        market["endDate"] = REVISED_END.into();
    }
    let condition = market["conditionId"].as_str().unwrap();
    let end = market["endDate"].as_str().unwrap();
    let matches = query.get("condition_ids").is_none_or(|ids| ids.contains(condition))
        // Both fixture dates and the configured bound use the same UTC representation.
        && query.get("end_date_max").is_none_or(|maximum| end <= maximum.as_str());
    venue.requests.lock().unwrap().push(json!({
        "after_revision": revised,
        "query": query,
        "returned_markets": usize::from(matches),
        "returned_end_date": matches.then_some(end),
    }));
    Json(json!({
        "markets": if matches { vec![market] } else { Vec::new() },
        "next_cursor": null,
    }))
}
