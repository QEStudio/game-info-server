use axum::{
    extract::{ConnectInfo, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc};

use crate::{access, utils, AppState};

#[derive(Deserialize, Default)]
pub struct GameInfoInput {
    #[serde(default)]
    pub uuid: String,
}

pub async fn handle_game_info(
    State(state): State<Arc<AppState>>,
    Path(custom_id): Path<u32>,
    headers: axum::http::HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: Option<Json<GameInfoInput>>,
) -> impl IntoResponse {
    let uuid = body.map(|Json(b)| b.uuid).unwrap_or_default();
    let ip   = utils::extract_ip(&headers, &addr.to_string());

    // 访问记录异步处理，不阻塞响应
    let state_ref = Arc::clone(&state);
    let ip2       = ip.clone();
    let uuid2     = uuid.clone();
    tokio::spawn(async move {
        access::record_access(&state_ref, &ip2, &uuid2).await;
    });

    // 游戏数据查询（内存缓存 O(1)）
    let entry = match state
        .version_cache
        .get_or_load(
            &state.config.images_dir,
            &state.config.version_cache_dir,
            custom_id,
        )
        .await
    {
        Some(e) => e,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "code": 404,
                    "message": format!("Game data not found for custom_id: {}", custom_id)
                })),
            )
                .into_response();
        }
    };

    let d = &entry.data;

    let game_info = json!({
        "game_name":         str_val(d, "game_name"),
        "kind_name":         str_val(d, "kind_name"),
        "version_name":      &entry.version_name,
        "game_id":           int_val(d, "game_id"),
        "version_id":        int_val(d, "version_id"),
        "custom_id":         custom_id,
        "custom_version_id": int_val(d, "custom_version_id"),
        "custom":            bool_val(d, "custom"),
        "description":       str_val(d, "description"),
        "link":              str_val(d, "link"),
    });

    (
        StatusCode::OK,
        Json(json!({
            "code": 200,
            "data": { "game_info": game_info }
        })),
    )
        .into_response()
}

pub async fn handle_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "code": 404, "message": "Not Found" })),
    )
}

#[inline]
fn str_val<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

#[inline]
fn int_val(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(0)
}

#[inline]
fn bool_val(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(true)
}