use std::{
    collections::HashMap,
    sync::{
        Arc,
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
    discovery_refreshed: Arc<AtomicBool>,
}

impl Venue {
    pub fn revise(&self) {
        self.revised.store(true, Ordering::SeqCst);
    }

    pub fn discovery_refreshed(&self) -> bool {
        self.discovery_refreshed.load(Ordering::SeqCst)
    }
}

pub async fn serve() -> Result<(Venue, String, tokio::task::JoinHandle<std::io::Result<()>>)> {
    let venue = Venue {
        market: serde_json::from_str(include_str!("../data/market.json"))?,
        revised: Arc::default(),
        discovery_refreshed: Arc::default(),
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
    if revised
        && query
            .get("end_date_max")
            .is_some_and(|maximum| maximum == DISCOVERY_END)
        && !matches
    {
        venue.discovery_refreshed.store(true, Ordering::SeqCst);
    }
    Json(json!({
        "markets": if matches { vec![market] } else { Vec::new() },
        "next_cursor": null,
    }))
}
