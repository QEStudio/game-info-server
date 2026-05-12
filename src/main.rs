#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod access;
mod cache;
mod config;
mod handlers;
mod state;
mod stats;
mod utils;

use axum::{middleware, routing::post, Router};
use std::{net::SocketAddr, sync::Arc};
use tokio::signal;
use tracing::info;

pub use state::AppState;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "info".into())
                .as_str()
                .parse::<tracing_subscriber::EnvFilter>()
                .unwrap(),
        )
        .with_target(false)
        .compact()
        .init();

    let config = config::Config::from_env();
    let bind_addr: SocketAddr = config.bind_addr.parse().expect("invalid BIND_ADDR");

    for dir in [
        &config.cache_dir,
        &config.version_cache_dir,
        &config.geoip_cache_dir,
        &config.users_dir,
    ] {
        tokio::fs::create_dir_all(dir)
            .await
            .unwrap_or_else(|e| tracing::warn!("mkdir {:?}: {}", dir, e));
    }

    let state = Arc::new(AppState::new(config));

    let geoip_state = Arc::clone(&state);
    tokio::spawn(async move {
        cache::geoip::run_geoip_worker(geoip_state).await;
    });

    let stats_state = Arc::clone(&state);
    tokio::spawn(async move {
        stats::run_flush_worker(stats_state).await;
    });

    let app = Router::new()
        .route(
            "/images/game_info/custom/new/:custom_id/data.json",
            post(handlers::handle_game_info),
        )
        .fallback(handlers::handle_not_found)
        .layer(middleware::from_fn(access_log_middleware))
        .with_state(Arc::clone(&state));

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .expect("bind failed");

    info!("listening on {}", bind_addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(Arc::clone(&state)))
    .await
    .expect("server error");
}

/// 访问日志中间件
async fn access_log_middleware(
    req: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let method = req.method().clone();
    let uri    = req.uri().clone();
    let start  = std::time::Instant::now();

    let ip = extract_ip_from_req(&req);

    let resp    = next.run(req).await;
    let status  = resp.status().as_u16();
    let elapsed = start.elapsed();

    let level = if status >= 500 {
        tracing::Level::ERROR
    } else if status >= 400 {
        tracing::Level::WARN
    } else {
        tracing::Level::INFO
    };

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");

    match level {
        tracing::Level::ERROR => tracing::error!(
            "[{}] {} \"{} {}\" {} {:.2}ms",
            now, ip, method, uri.path(), status,
            elapsed.as_secs_f64() * 1000.0,
        ),
        tracing::Level::WARN => tracing::warn!(
            "[{}] {} \"{} {}\" {} {:.2}ms",
            now, ip, method, uri.path(), status,
            elapsed.as_secs_f64() * 1000.0,
        ),
        _ => tracing::info!(
            "[{}] {} \"{} {}\" {} {:.2}ms",
            now, ip, method, uri.path(), status,
            elapsed.as_secs_f64() * 1000.0,
        ),
    }

    resp
}

fn extract_ip_from_req(req: &axum::extract::Request) -> String {
    let headers = req.headers();

    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = v.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }

    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        return v.trim().to_string();
    }

    "-".to_string()
}

async fn shutdown_signal(state: Arc<AppState>) {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to listen ctrl_c");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown: flushing stats...");
    stats::force_flush(&state).await;
    info!("shutdown complete");
}