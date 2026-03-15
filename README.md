# Massive Boids Stream

10万点のBoidsシミュレーションをサーバーで事前計算し、動画としてブラウザで再生するWebアプリ。

## Features

- **大規模シミュレーション**: 10万点のBoids粒子
- **オフラインレンダリング**: Rust + Rayonで並列計算、FFmpegで動画化
- **視覚効果**: 残像フェード、速度に応じたカラーマッピング（青→オレンジ）
- **リアルタイム進捗**: SSEでレンダリング進捗を通知
- **動画仕様**: 30秒、30fps、720×1280（9:16縦長）

## Quick Start

```bash
docker compose up --build
```

- **フロントエンド**: http://localhost:8889
- **バックエンドAPI**: http://localhost:8888

初回アクセス時に自動でシミュレーションが開始され、完了後に動画が再生されます。

## Architecture

```
┌─────────────────┐     ┌─────────────────┐
│    Frontend     │────▶│     Nginx       │
│  (Browser)      │◀────│   :8889/80      │
└─────────────────┘     └────────┬────────┘
                                 │
              ┌──────────────────┼──────────────────┐
              │                  │                  │
              ▼                  ▼                  ▼
        /api/status        /api/progress      /video/boids.mp4
        /api/generate         (SSE)
              │                  │                  │
              └──────────────────┼──────────────────┘
                                 │
                        ┌────────▼────────┐
                        │  Rust Backend   │
                        │    :3000        │
                        └────────┬────────┘
                                 │
              ┌──────────────────┼──────────────────┐
              │                  │                  │
              ▼                  ▼                  ▼
         Boids Sim          Rendering          FFmpeg
         (Rayon)            (image)            Encoding
```

## Tech Stack

| Component | Technology | Purpose |
|-----------|------------|---------|
| Backend | Rust + Actix-web | HTTP Server, SSE |
| Parallelization | Rayon | Multi-threaded simulation |
| Rendering | image crate | Frame generation |
| Encoding | FFmpeg | Video creation |
| Frontend | Vanilla JS | Progress display, video playback |
| Server | Nginx | Static files, reverse proxy |
| Container | Docker | Deployment |
