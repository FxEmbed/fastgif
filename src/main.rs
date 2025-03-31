use anyhow::{anyhow, Result};
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use std::{
    process::{Command, Stdio},
};
use tempfile::NamedTempFile;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command as TokioCommand,
};
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use reqwest;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();
    info!("Starting FastGIF server");

    // Check if FFmpeg is available
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(_) => info!("FFmpeg is available"),
        Err(e) => {
            error!("FFmpeg not found: {}", e);
            return Err(anyhow!("FFmpeg not found, please install FFmpeg"));
        }
    }

    // Check if gifski is available
    match Command::new("gifski").arg("--version").output() {
        Ok(_) => info!("gifski is available"),
        Err(e) => {
            error!("gifski not found: {}", e);
            return Err(anyhow!("gifski not found, please install gifski"));
        }
    }

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
                [("Content-Type", "image/gif")],
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
    info!("Downloading video from {}", video_url);

    // First download the video file to a temporary file with proper extension
    let client = reqwest::Client::new();
    let response = client.get(&video_url)
        .send()
        .await?;
    
    if !response.status().is_success() {
        return Err(anyhow!("Failed to download video, status: {}", response.status()));
    }
    
    // Determine extension from content type
    let content_type = response.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("video/mp4");
    
    let extension = if content_type.contains("mp4") {
        "mp4"
    } else if content_type.contains("webm") {
        "webm"
    } else {
        "mp4" // Default to mp4
    };
    
    // Save the video to a temporary file with proper extension
    let temp_dir = tempfile::tempdir()?;
    let video_path = temp_dir.path().join(format!("input.{}", extension));
    let video_path_str = video_path.to_str().ok_or_else(|| anyhow!("Invalid path"))?;
    
    let video_data = response.bytes().await?;
    let mut video_file = tokio::fs::File::create(&video_path).await?;
    tokio::io::copy(&mut &*video_data, &mut video_file).await?;
    
    info!("Video downloaded to temporary file: {}", video_path_str);
    
    // Now process the local file
    process_local_video_to_gif(video_path_str).await
}

async fn process_local_video_to_gif(video_path: &str) -> Result<Bytes> {
    info!("Converting local video to GIF: {}", video_path);

    // Create a temporary file for the GIF output with proper extension
    let temp_dir = tempfile::tempdir()?;
    let gif_path = temp_dir.path().join("output.gif");
    let gif_path_str = gif_path.to_str().ok_or_else(|| anyhow!("Invalid path"))?;

    // Use gifski directly on the video file (gifski uses FFmpeg internally)
    let mut gifski_process = TokioCommand::new("gifski")
        .args([
            "--quality", "90",
            "--fps", "25",
            "--output", gif_path_str,
            video_path,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
    let mut gifski_stderr = gifski_process.stderr.take()
        .ok_or_else(|| anyhow!("Failed to get gifski stderr"))?;

    // Collect stderr from gifski process
    let gifski_stderr_future = async {
        let mut buffer = Vec::new();
        gifski_stderr.read_to_end(&mut buffer).await.unwrap_or(0);
        String::from_utf8_lossy(&buffer).to_string()
    };

    // Wait for gifski to finish
    let gifski_status = gifski_process.wait().await?;
    let gifski_stderr_output = gifski_stderr_future.await;
    
    if !gifski_status.success() {
        error!("gifski stderr: {}", gifski_stderr_output);
        return Err(anyhow!("gifski process failed: {}", gifski_stderr_output));
    }

    // Read the GIF file into memory
    let mut gif_file = tokio::fs::File::open(gif_path_str).await?;
    let mut gif_data = Vec::new();
    gif_file.read_to_end(&mut gif_data).await?;

    Ok(Bytes::from(gif_data))
}
