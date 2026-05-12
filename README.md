# game-info-server

高性能游戏信息查询服务，Rust 实现，对标并替换原 PHP 版本。

## 功能

- 根据 `custom_id` 返回最新版本的游戏数据
- 记录用户访问（IP、UUID、地理位置、访问次数）
- 异步 GeoIP 查询（ip-api.com），两级缓存（内存 + 磁盘）
- 访问统计批量落盘，支持优雅关闭强制刷盘

## 性能对比

| 指标         | PHP（FPM）         | Rust（本服务）         |
|--------------|--------------------|------------------------|
| 并发模型     | 多进程，每请求阻塞  | 异步多线程，无阻塞      |
| 内存占用     | ~30 MB/worker      | 空载 ~4 MB，全程 <30 MB|
| 二进制大小   | N/A                | ~3 MB（strip + LTO）   |
| 版本缓存     | 每次读文件 + 解析  | 内存 Arc 共享，O(1)    |
| GeoIP        | 请求内同步阻塞查询  | 后台 Worker 异步查询   |
| 统计写盘     | 每请求加锁写文件   | 原子计数 + 阈值批量写  |

## 目录结构

```
.
├── Cargo.toml
├── README.md
└── src/
    ├── main.rs          # 入口、路由、优雅关闭
    ├── config.rs        # 环境变量配置
    ├── state.rs         # 全局共享状态
    ├── handlers.rs      # HTTP 请求处理
    ├── access.rs        # 用户访问记录
    ├── stats.rs         # 访问统计（原子计数 + 批量落盘）
    ├── utils.rs         # IP 提取等工具函数
    └── cache/
        ├── mod.rs
        ├── version.rs   # 游戏版本两级缓存
        └── geoip.rs     # GeoIP 两级缓存 + 后台 Worker
```

## 数据目录结构

```
data/
├── images/
│   └── {custom_id}/
│       └── {version}/
│           └── data.json          # 游戏数据文件
│
├── users/
│   ├── {uuid}/                    # 有 UUID 的用户（按 UUID 隔离）
│   │   ├── geo.json               # 地理位置（含原始 IP）
│   │   └── times.txt              # 访问记录
│   └── public/
│       └── {ip}/                  # 无 UUID 的用户（按 IP 隔离）
│           ├── geo.json
│           └── times.txt
│
└── cache/
    ├── stats.json                 # 全局访问统计
    ├── versions/
    │   └── {custom_id}.json       # 版本数据磁盘缓存
    └── geoip/
        └── {ip}.json              # GeoIP 磁盘缓存
```

### data.json 格式示例

```json
{
    "game_name": "示例游戏",
    "kind_name": "示例类型",
    "game_id": 1001,
    "version_id": 2001,
    "custom_version_id": 3001,
    "custom": true,
    "description": "游戏描述",
    "link": "https://example.com"
}
```

### geo.json 格式

```json
{
    "ip": "1.2.3.4",
    "country": "China",
    "country_code": "CN",
    "city": "Beijing",
    "lat": 39.9042,
    "lon": 116.4074,
    "timezone": "Asia/Shanghai"
}
```

### times.txt 格式

```
42          ← 访问次数
1700000000  ← 首次访问 Unix 时间戳
1700100000  ← 最近访问 Unix 时间戳
```

### stats.json 格式

```json
{
    "total_users": 1000,
    "total_access": 5000,
    "with_uuid": 800,
    "without_uuid": 200,
    "regions": {
        "CN": { "country": "China", "count": 600 },
        "US": { "country": "United States", "count": 200 }
    },
    "last_updated": "unix:1700000000"
}
```

## 接口文档

### 查询游戏信息

```
POST /images/game_info/custom/new/{custom_id}/data.json
Content-Type: application/json
```

**请求体（可选）**

```json
{
    "uuid": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
}
```

| 字段   | 类型   | 说明                        |
|--------|--------|-----------------------------|
| `uuid` | string | 客户端唯一标识，标准 UUID v4 格式，可不传 |

**成功响应 200**

```json
{
    "code": 200,
    "data": {
        "game_info": {
            "game_name": "示例游戏",
            "kind_name": "示例类型",
            "version_name": "1.2.3",
            "game_id": 1001,
            "version_id": 2001,
            "custom_id": 10,
            "custom_version_id": 3001,
            "custom": true,
            "description": "游戏描述",
            "link": "https://example.com"
        }
    }
}
```

**失败响应 404**

```json
{
    "code": 404,
    "message": "Game data not found for custom_id: 10"
}
```

## 环境变量

| 变量名           | 默认值          | 说明                                         |
|------------------|-----------------|----------------------------------------------|
| `DATA_DIR`       | `../data`       | 数据根目录                                   |
| `BIND_ADDR`      | `0.0.0.0:3000`  | 监听地址和端口                               |
| `FLUSH_INTERVAL` | `10`            | 累积多少次访问后触发统计刷盘                 |
| `GEOIP_WORKERS`  | `3`             | GeoIP 后台并发查询数                         |
| `RUST_LOG`       | `info`          | 日志级别（`error/warn/info/debug/trace`）    |

## 构建

**环境要求**

- Rust 1.75+
- 联网（构建时下载依赖）

```bash
# 开发构建
cargo build

# 生产构建（最小体积、最高性能）
cargo build --release
```

生产二进制位于 `target/release/game-info-server`，大小约 3 MB。

## 运行

```bash
# 直接运行
DATA_DIR=/data \
BIND_ADDR=0.0.0.0:3000 \
FLUSH_INTERVAL=10 \
GEOIP_WORKERS=3 \
RUST_LOG=info \
./target/release/game-info-server
```

## systemd 部署

`/etc/systemd/system/game-info-server.service`

```ini
[Unit]
Description=Game Info Server
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/opt/game-info-server
ExecStart=/opt/game-info-server/game-info-server
Restart=always
RestartSec=5

Environment=DATA_DIR=/data
Environment=BIND_ADDR=0.0.0.0:3000
Environment=FLUSH_INTERVAL=10
Environment=GEOIP_WORKERS=3
Environment=RUST_LOG=info

# 资源限制
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload
systemctl enable game-info-server
systemctl start game-info-server
systemctl status game-info-server
```

## Nginx 反代配置

```nginx
upstream game_info {
    server 127.0.0.1:3000;
    keepalive 32;
}

server {
    listen 80;
    server_name your.domain.com;

    location /images/game_info/ {
        proxy_pass         http://game_info;
        proxy_http_version 1.1;
        proxy_set_header   Connection      "";
        proxy_set_header   Host            $host;
        proxy_set_header   X-Real-IP       $remote_addr;
        proxy_set_header   X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_read_timeout 10s;
    }
}
```

## GeoIP 查询说明

- 使用 [ip-api.com](http://ip-api.com) 免费接口（非商业用途，限速 45 次/分钟）
- 查询结果永久缓存到磁盘，同一 IP 只查询一次
- 私有/本地 IP（`127.x`、`192.168.x`、`10.x`、`172.x`）直接标记为 `Local`，不发起请求
- 查询失败时返回 `Unknown` 占位，下次请求时自动重试

## 优雅关闭

收到 `SIGTERM` 或 `Ctrl+C` 时：

1. 停止接受新连接
2. 等待当前请求处理完毕
3. 强制将内存中的统计计数刷盘
4. 退出