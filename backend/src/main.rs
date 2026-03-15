use actix_files::NamedFile;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use futures::stream::{Stream, StreamExt};
use image::{ImageBuffer, Rgb, RgbImage};
use log::{error, info};
use rand::Rng;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

// ============================================================================
// Constants
// ============================================================================

const WIDTH: u32 = 720;
const HEIGHT: u32 = 1280;
const NUM_BOIDS: usize = 100_000;
const FPS: u32 = 30;
const DURATION_SECS: u32 = 30;
const TOTAL_FRAMES: u32 = FPS * DURATION_SECS;

// Boids parameters
const MAX_SPEED: f32 = 4.5;
const MIN_SPEED: f32 = 2.0;
const SEPARATION_RADIUS: f32 = 8.0;
const ALIGNMENT_RADIUS: f32 = 30.0;
const COHESION_RADIUS: f32 = 45.0;
const SEPARATION_WEIGHT: f32 = 1.2;
const ALIGNMENT_WEIGHT: f32 = 1.0;
const COHESION_WEIGHT: f32 = 1.6;

// Visual parameters
const FADE_FACTOR: f32 = 0.85;
const PARTICLE_SIZE: i32 = 1;  // 3x3 pixels per boid

// Grid cell size for spatial partitioning
const CELL_SIZE: f32 = 30.0;

// Output directory
const OUTPUT_DIR: &str = "/app/output";
const VIDEO_PATH: &str = "/app/output/boids.mp4";

// ============================================================================
// Boid Structure
// ============================================================================

#[derive(Clone, Copy)]
struct Boid {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

impl Boid {
    fn new_random(width: f32, height: f32) -> Self {
        let mut rng = rand::thread_rng();
        let angle = rng.gen::<f32>() * std::f32::consts::TAU;
        let speed = rng.gen_range(MIN_SPEED..MAX_SPEED);
        Boid {
            x: rng.gen::<f32>() * width,
            y: rng.gen::<f32>() * height,
            vx: angle.cos() * speed,
            vy: angle.sin() * speed,
        }
    }

    fn speed(&self) -> f32 {
        (self.vx * self.vx + self.vy * self.vy).sqrt()
    }
}

// ============================================================================
// Spatial Grid for efficient neighbor lookup
// ============================================================================

struct SpatialGrid {
    cells: HashMap<(i32, i32), Vec<usize>>,
    cell_size: f32,
}

impl SpatialGrid {
    fn new(cell_size: f32) -> Self {
        SpatialGrid {
            cells: HashMap::new(),
            cell_size,
        }
    }

    fn clear(&mut self) {
        self.cells.clear();
    }

    fn insert(&mut self, idx: usize, x: f32, y: f32) {
        let cell_x = (x / self.cell_size).floor() as i32;
        let cell_y = (y / self.cell_size).floor() as i32;
        self.cells.entry((cell_x, cell_y)).or_default().push(idx);
    }

    fn get_neighbors(&self, x: f32, y: f32, radius: f32) -> Vec<usize> {
        let cell_x = (x / self.cell_size).floor() as i32;
        let cell_y = (y / self.cell_size).floor() as i32;
        let cell_radius = (radius / self.cell_size).ceil() as i32;

        let mut neighbors = Vec::new();
        for dx in -cell_radius..=cell_radius {
            for dy in -cell_radius..=cell_radius {
                if let Some(indices) = self.cells.get(&(cell_x + dx, cell_y + dy)) {
                    neighbors.extend(indices.iter().copied());
                }
            }
        }
        neighbors
    }
}

// ============================================================================
// Boids Simulation
// ============================================================================

fn update_boids(boids: &[Boid], grid: &SpatialGrid, cohesion_multiplier: f32) -> Vec<Boid> {
    let width = WIDTH as f32;
    let height = HEIGHT as f32;
    let max_radius = COHESION_RADIUS.max(ALIGNMENT_RADIUS).max(SEPARATION_RADIUS);

    boids
        .par_iter()
        .enumerate()
        .map(|(i, boid)| {
            let neighbor_indices = grid.get_neighbors(boid.x, boid.y, max_radius);

            let mut sep_x = 0.0f32;
            let mut sep_y = 0.0f32;
            let mut sep_count = 0;

            let mut align_vx = 0.0f32;
            let mut align_vy = 0.0f32;
            let mut align_count = 0;

            let mut coh_x = 0.0f32;
            let mut coh_y = 0.0f32;
            let mut coh_count = 0;

            for &j in &neighbor_indices {
                if i == j {
                    continue;
                }
                let other = &boids[j];
                let dx = boid.x - other.x;
                let dy = boid.y - other.y;
                let dist_sq = dx * dx + dy * dy;

                // Separation
                if dist_sq < SEPARATION_RADIUS * SEPARATION_RADIUS && dist_sq > 0.0 {
                    let dist = dist_sq.sqrt();
                    sep_x += dx / dist;
                    sep_y += dy / dist;
                    sep_count += 1;
                }

                // Alignment
                if dist_sq < ALIGNMENT_RADIUS * ALIGNMENT_RADIUS {
                    align_vx += other.vx;
                    align_vy += other.vy;
                    align_count += 1;
                }

                // Cohesion
                if dist_sq < COHESION_RADIUS * COHESION_RADIUS {
                    coh_x += other.x;
                    coh_y += other.y;
                    coh_count += 1;
                }
            }

            let mut new_vx = boid.vx;
            let mut new_vy = boid.vy;

            // Apply separation
            if sep_count > 0 {
                new_vx += (sep_x / sep_count as f32) * SEPARATION_WEIGHT;
                new_vy += (sep_y / sep_count as f32) * SEPARATION_WEIGHT;
            }

            // Apply alignment (also scales with cohesion_multiplier)
            if align_count > 0 {
                let avg_vx = align_vx / align_count as f32;
                let avg_vy = align_vy / align_count as f32;
                new_vx += (avg_vx - boid.vx) * ALIGNMENT_WEIGHT * 0.05 * cohesion_multiplier;
                new_vy += (avg_vy - boid.vy) * ALIGNMENT_WEIGHT * 0.05 * cohesion_multiplier;
            }

            // Apply cohesion (scales with time)
            if coh_count > 0 {
                let center_x = coh_x / coh_count as f32;
                let center_y = coh_y / coh_count as f32;
                new_vx += (center_x - boid.x) * COHESION_WEIGHT * 0.005 * cohesion_multiplier;
                new_vy += (center_y - boid.y) * COHESION_WEIGHT * 0.005 * cohesion_multiplier;
            }

            // Clamp speed
            let speed = (new_vx * new_vx + new_vy * new_vy).sqrt();
            if speed > MAX_SPEED {
                new_vx = new_vx / speed * MAX_SPEED;
                new_vy = new_vy / speed * MAX_SPEED;
            } else if speed < MIN_SPEED && speed > 0.0 {
                new_vx = new_vx / speed * MIN_SPEED;
                new_vy = new_vy / speed * MIN_SPEED;
            }

            // Update position with wrapping
            let mut new_x = boid.x + new_vx;
            let mut new_y = boid.y + new_vy;

            // Wrap around edges
            if new_x < 0.0 {
                new_x += width;
            } else if new_x >= width {
                new_x -= width;
            }
            if new_y < 0.0 {
                new_y += height;
            } else if new_y >= height {
                new_y -= height;
            }

            Boid {
                x: new_x,
                y: new_y,
                vx: new_vx,
                vy: new_vy,
            }
        })
        .collect()
}

// ============================================================================
// Rendering
// ============================================================================

fn speed_to_color(speed: f32) -> Rgb<u8> {
    // Map speed from MIN_SPEED..MAX_SPEED to dark blue..bright cyan
    let t = ((speed - MIN_SPEED) / (MAX_SPEED - MIN_SPEED)).clamp(0.0, 1.0);

    // Dark blue (low speed): RGB(20, 60, 140)
    // Bright cyan (high speed): RGB(100, 220, 255)
    let r = (20.0 + t * (100.0 - 20.0)) as u8;
    let g = (60.0 + t * (220.0 - 60.0)) as u8;
    let b = (140.0 + t * (255.0 - 140.0)) as u8;

    Rgb([r, g, b])
}

fn render_frame(boids: &[Boid], canvas: &mut RgbImage) {
    // Apply fade effect (darken previous frame)
    for pixel in canvas.pixels_mut() {
        pixel[0] = (pixel[0] as f32 * FADE_FACTOR) as u8;
        pixel[1] = (pixel[1] as f32 * FADE_FACTOR) as u8;
        pixel[2] = (pixel[2] as f32 * FADE_FACTOR) as u8;
    }

    // Draw boids
    for boid in boids {
        let color = speed_to_color(boid.speed());
        let x = boid.x as i32;
        let y = boid.y as i32;

        // Draw a small square for each boid
        for dy in -PARTICLE_SIZE..=PARTICLE_SIZE {
            for dx in -PARTICLE_SIZE..=PARTICLE_SIZE {
                let px = x + dx;
                let py = y + dy;
                if px >= 0 && px < WIDTH as i32 && py >= 0 && py < HEIGHT as i32 {
                    canvas.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}

// ============================================================================
// Video Generation
// ============================================================================

fn generate_video(progress_sender: broadcast::Sender<ProgressEvent>) -> Result<(), String> {
    info!("Starting video generation...");

    // Initialize boids
    let width = WIDTH as f32;
    let height = HEIGHT as f32;
    let mut boids: Vec<Boid> = (0..NUM_BOIDS).map(|_| Boid::new_random(width, height)).collect();

    // Initialize spatial grid
    let mut grid = SpatialGrid::new(CELL_SIZE);

    // Initialize canvas
    let mut canvas: RgbImage = ImageBuffer::new(WIDTH, HEIGHT);

    // Ensure output directory exists
    std::fs::create_dir_all(OUTPUT_DIR).map_err(|e| format!("Failed to create output dir: {}", e))?;

    // Start FFmpeg process
    let mut ffmpeg = Command::new("ffmpeg")
        .args([
            "-y",                        // Overwrite output
            "-f", "rawvideo",
            "-vcodec", "rawvideo",
            "-pix_fmt", "rgb24",
            "-s", &format!("{}x{}", WIDTH, HEIGHT),
            "-r", &FPS.to_string(),
            "-i", "-",                   // Read from stdin
            "-c:v", "libx264",
            "-preset", "fast",
            "-crf", "23",
            "-pix_fmt", "yuv420p",
            VIDEO_PATH,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start FFmpeg: {}", e))?;

    let mut stdin = ffmpeg.stdin.take().ok_or("Failed to get FFmpeg stdin")?;

    // Generate frames
    for frame in 0..TOTAL_FRAMES {
        // Update spatial grid
        grid.clear();
        for (i, boid) in boids.iter().enumerate() {
            grid.insert(i, boid.x, boid.y);
        }

        // Calculate cohesion multiplier: starts at 0.1, reaches 1.0 at frame 300 (10 seconds)
        let ramp_frames = 300.0;
        let cohesion_multiplier = (0.1 + 0.9 * (frame as f32 / ramp_frames).min(1.0)).min(1.0);

        // Update boids
        boids = update_boids(&boids, &grid, cohesion_multiplier);

        // Render frame
        render_frame(&boids, &mut canvas);

        // Write frame to FFmpeg
        stdin
            .write_all(canvas.as_raw())
            .map_err(|e| format!("Failed to write frame: {}", e))?;

        // Send progress update
        let progress = ((frame + 1) as f32 / TOTAL_FRAMES as f32 * 100.0) as u32;
        let _ = progress_sender.send(ProgressEvent {
            progress,
            frame: frame + 1,
            total_frames: TOTAL_FRAMES,
            status: if frame + 1 == TOTAL_FRAMES {
                "complete".to_string()
            } else {
                "rendering".to_string()
            },
        });

        if frame % 30 == 0 {
            info!("Frame {}/{} ({}%)", frame + 1, TOTAL_FRAMES, progress);
        }
    }

    // Close stdin and wait for FFmpeg to finish
    drop(stdin);
    let status = ffmpeg.wait().map_err(|e| format!("FFmpeg error: {}", e))?;

    if !status.success() {
        return Err("FFmpeg encoding failed".to_string());
    }

    info!("Video generation complete: {}", VIDEO_PATH);
    Ok(())
}

// ============================================================================
// Server State and SSE
// ============================================================================

#[derive(Clone, Serialize)]
struct ProgressEvent {
    progress: u32,
    frame: u32,
    total_frames: u32,
    status: String,
}

struct AppState {
    progress_sender: broadcast::Sender<ProgressEvent>,
    is_generating: AtomicBool,
    generation_complete: AtomicBool,
    current_progress: AtomicU32,
}

fn create_sse_stream(receiver: broadcast::Receiver<ProgressEvent>) -> impl Stream<Item = Result<web::Bytes, actix_web::Error>> {
    BroadcastStream::new(receiver)
        .filter_map(|result| async move {
            match result {
                Ok(event) => {
                    let data = format!(
                        "data: {}\n\n",
                        serde_json::to_string(&event).unwrap_or_default()
                    );
                    Some(Ok(web::Bytes::from(data)))
                }
                Err(_) => None,
            }
        })
}

// ============================================================================
// HTTP Handlers
// ============================================================================

async fn sse_progress(data: web::Data<Arc<AppState>>) -> impl Responder {
    // If already complete, send final event immediately
    if data.generation_complete.load(Ordering::SeqCst) {
        let body = format!(
            "data: {}\n\n",
            serde_json::to_string(&ProgressEvent {
                progress: 100,
                frame: TOTAL_FRAMES,
                total_frames: TOTAL_FRAMES,
                status: "complete".to_string(),
            })
            .unwrap()
        );
        return HttpResponse::Ok()
            .insert_header(("Content-Type", "text/event-stream"))
            .insert_header(("Cache-Control", "no-cache"))
            .body(body);
    }

    let receiver = data.progress_sender.subscribe();
    let stream = create_sse_stream(receiver);

    HttpResponse::Ok()
        .insert_header(("Content-Type", "text/event-stream"))
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("Connection", "keep-alive"))
        .streaming(stream)
}

async fn get_status(data: web::Data<Arc<AppState>>) -> impl Responder {
    let is_complete = data.generation_complete.load(Ordering::SeqCst);
    let is_generating = data.is_generating.load(Ordering::SeqCst);
    let progress = data.current_progress.load(Ordering::SeqCst);

    HttpResponse::Ok().json(serde_json::json!({
        "generating": is_generating,
        "complete": is_complete,
        "progress": progress,
        "video_ready": is_complete && Path::new(VIDEO_PATH).exists()
    }))
}

async fn serve_video(_req: HttpRequest) -> actix_web::Result<NamedFile> {
    let file = NamedFile::open(VIDEO_PATH)?
        .use_etag(true)
        .use_last_modified(true);
    Ok(file)
}

async fn start_generation(data: web::Data<Arc<AppState>>) -> impl Responder {
    // Check if already generating or complete
    if data.is_generating.load(Ordering::SeqCst) {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": "Generation already in progress"
        }));
    }

    if data.generation_complete.load(Ordering::SeqCst) && Path::new(VIDEO_PATH).exists() {
        return HttpResponse::Ok().json(serde_json::json!({
            "status": "already_complete",
            "video_ready": true
        }));
    }

    data.is_generating.store(true, Ordering::SeqCst);
    let sender = data.progress_sender.clone();
    let state = data.clone();

    // Spawn video generation in background
    tokio::task::spawn_blocking(move || {
        match generate_video(sender) {
            Ok(_) => {
                state.generation_complete.store(true, Ordering::SeqCst);
                state.current_progress.store(100, Ordering::SeqCst);
            }
            Err(e) => {
                error!("Video generation failed: {}", e);
            }
        }
        state.is_generating.store(false, Ordering::SeqCst);
    });

    HttpResponse::Ok().json(serde_json::json!({
        "status": "started"
    }))
}

// ============================================================================
// Main
// ============================================================================

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();
    info!("Starting Boids server...");

    let (tx, _) = broadcast::channel::<ProgressEvent>(100);

    let state = Arc::new(AppState {
        progress_sender: tx,
        is_generating: AtomicBool::new(false),
        generation_complete: AtomicBool::new(Path::new(VIDEO_PATH).exists()),
        current_progress: AtomicU32::new(0),
    });

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .route("/api/status", web::get().to(get_status))
            .route("/api/generate", web::post().to(start_generation))
            .route("/api/progress", web::get().to(sse_progress))
            .route("/video/boids.mp4", web::get().to(serve_video))
    })
    .bind("0.0.0.0:3000")?
    .run()
    .await
}
