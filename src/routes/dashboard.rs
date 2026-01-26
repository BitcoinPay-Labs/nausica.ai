use axum::{
    extract::State,
    response::{Html, Json},
};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::JobSummary;
use crate::AppState;

pub async fn dashboard_page() -> Html<String> {
    Html(include_str!("../../templates/dashboard.html").to_string())
}

pub async fn get_jobs(
    State(state): State<Arc<RwLock<AppState>>>,
) -> Json<Vec<JobSummary>> {
    let state = state.read().await;
    let jobs = state.db.get_all_jobs().unwrap_or_default();
    Json(jobs)
}
