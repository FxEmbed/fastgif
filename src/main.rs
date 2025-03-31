use anyhow::{anyhow, Result};
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use std::process::Stdio;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command as TokioCommand,
};
use tower_http::trace::TraceLayer;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();
    info!("Starting FastGIF server");
    // Build our application with a route
    let app = Router::new()
        .route("/*path", get(handle_tweet_video))
        .layer(TraceLayer::new_for_http());

    // Run it with hyper on localhost:3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!("Listening on http://localhost:3000");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_tweet_video(Path(path): Path<String>) -> Response {
    info!("Processing video: {}", path);
    
    match process_video_to_gif(&path).await {
        Ok(gif_data) => {
            info!("Successfully converted video to GIF");
            
            (
                StatusCode::OK,
                [("Content-Type", "image/gif"), ("Cache-Control", "public, max-age=31536000")],
                gif_data,
            )
                .into_response()
        }
        Err(e) => {
            error!("Failed to process video: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to process video: {}", e),
            )
                .into_response()
        }
    }
}

async fn process_video_to_gif(path: &str) -> Result<Bytes> {
    let video_url = format!("https://video.twimg.com/{}", path);
    info!("Processing video from URL: {}", video_url);
    
    // Try the direct FFmpeg download and pipe approach
    process_using_direct_url(&video_url).await
}

async fn process_using_direct_url(video_url: &str) -> Result<Bytes> {
    info!("Downloading video from {}", video_url);

    // Set up FFmpeg process to read directly from the URL and output yuv4mpegpipe
    let mut ffmpeg_process = TokioCommand::new("ffmpeg")
        .args([
            "-i", video_url,        // Read directly from URL
            "-f", "yuv4mpegpipe",   // Output in yuv4mpegpipe format
            "-"                     // Output to stdout
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
    // Set up gifski process to read yuv4mpegpipe frames from stdin and output to stdout
    let mut gifski_process = TokioCommand::new("gifski")
        .args([
            "--fast",              // Fast encoding
            "--output", "/dev/stdout",       // Output to stdout
            "-"                    // Read from stdin
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
    // Get stdin handle for gifski
    let mut gifski_stdin = gifski_process.stdin.take()
        .ok_or_else(|| anyhow!("Failed to get gifski stdin"))?;
    
    // Get stdout handle from ffmpeg
    let mut ffmpeg_stdout = ffmpeg_process.stdout.take()
        .ok_or_else(|| anyhow!("Failed to get ffmpeg stdout"))?;
    
    // Get output handle from gifski
    let mut gifski_stdout = gifski_process.stdout.take()
        .ok_or_else(|| anyhow!("Failed to get gifski stdout"))?;
    
    // Pipe data from ffmpeg stdout to gifski stdin - this is done in the main thread
    // to avoid the complexity of task management
    info!("Starting to pipe data from FFmpeg to gifski");
    
    // Use a heap-allocated buffer to avoid stack overflow
    let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer on the heap
    let mut total_bytes = 0;
    
    loop {
        match ffmpeg_stdout.read(&mut buffer).await {
            Ok(0) => {
                info!("Reached end of FFmpeg output stream after {} bytes", total_bytes);
                break; // EOF
            },
            Ok(n) => {
                total_bytes += n;
                if total_bytes % (1 * 1024 * 1024) == 0 { // Log every 1MB
                    info!("Piped {} MB from FFmpeg to gifski", total_bytes / (1024 * 1024));
                }
                
                gifski_stdin.write_all(&buffer[0..n]).await?;
            },
            Err(e) => return Err(anyhow!("Failed to read from ffmpeg stdout: {}", e)),
        }
    }
    
    // Make sure to close stdin so gifski knows we're done
    info!("Finished piping data, closing gifski stdin");
    drop(gifski_stdin);
    
    // Collect the output from gifski
    info!("Starting to collect gifski output");
    let mut gif_data = Vec::new();
    gifski_stdout.read_to_end(&mut gif_data).await?;
    info!("Collected {} bytes of GIF data from gifski", gif_data.len());
    
    // Wait for processes to complete
    let ffmpeg_status = ffmpeg_process.wait().await?;
    let gifski_status = gifski_process.wait().await?;
    
    // Check process exit statuses
    if !ffmpeg_status.success() {
        let mut buffer = Vec::new();
        if let Ok(stderr) = ffmpeg_process.stderr.unwrap().read_to_end(&mut buffer).await {
            return Err(anyhow!("FFmpeg process failed: {}", String::from_utf8_lossy(&buffer)));
        }
        return Err(anyhow!("FFmpeg process failed with no stderr output"));
    }
    
    if !gifski_status.success() {
        let mut buffer = Vec::new();
        if let Ok(stderr) = gifski_process.stderr.unwrap().read_to_end(&mut buffer).await {
            return Err(anyhow!("gifski process failed: {}", String::from_utf8_lossy(&buffer)));
        }
        return Err(anyhow!("gifski process failed with no stderr output"));
    }
    
    info!("Successfully generated GIF with {} bytes", gif_data.len());
    Ok(Bytes::from(gif_data))
}
