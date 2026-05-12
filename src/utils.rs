use axum::http::HeaderMap;

/// 提取真实客户端 IP
pub fn extract_ip(headers: &HeaderMap, remote_addr: &str) -> String {
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = v.split(',').next().unwrap_or("").trim();
        if !first.is_empty() && is_valid_ip_str(first) {
            return first.to_string();
        }
    }

    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if is_valid_ip_str(v) {
            return v.to_string();
        }
    }

    // 去掉端口，如 "1.2.3.4:56789" -> "1.2.3.4"
    if let Some(ip) = remote_addr.rsplit(':').nth(1) {
        return ip.trim_start_matches('[')
                 .trim_end_matches(']')
                 .to_string();
    }

    remote_addr.to_string()
}

#[inline]
fn is_valid_ip_str(s: &str) -> bool {
    if s.split('.').count() == 4 {
        return s.split('.').all(|p| p.parse::<u8>().is_ok());
    }
    s.contains(':')
}