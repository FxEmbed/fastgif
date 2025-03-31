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
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    info!("hello world!");
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
    info!("Converting directly from URL using FFmpeg and gifski with yuv4mpegpipe");

    // Set up FFmpeg process to read directly from the URL and output yuv4mpegpipe
    let mut ffmpeg_process = TokioCommand::new("ffmpeg")
        .args([
            "-i", video_url,        // Read directly from URL
            "-f", "yuv4mpegpipe",   // Output in yuv4mpegpipe format
            "-"                // Output to stdout
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
    // Set up gifski process to read yuv4mpegpipe frames from stdin
    let mut gifski_process = TokioCommand::new("gifski")
        .args([
            "--quality", "90",
            "--fps", "25",
            "--fast",
            "-o",
            "-"
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
    
    // Get stderr handles for logging
    let mut ffmpeg_stderr = ffmpeg_process.stderr.take()
        .ok_or_else(|| anyhow!("Failed to get ffmpeg stderr"))?;
    let mut gifski_stderr = gifski_process.stderr.take()
        .ok_or_else(|| anyhow!("Failed to get gifski stderr"))?;
    
    // Pipe data from ffmpeg stdout to gifski stdin
    let pipe_task = tokio::spawn(async move {
        let mut buffer = [0u8; 65536]; // 64KB buffer
        loop {
            match ffmpeg_stdout.read(&mut buffer).await {
                Ok(0) => break, // EOF
                Ok(n) => {
                    if let Err(e) = gifski_stdin.write_all(&buffer[0..n]).await {
                        return Err(anyhow!("Failed to write to gifski stdin: {}", e));
                    }
                },
                Err(e) => return Err(anyhow!("Failed to read from ffmpeg stdout: {}", e)),
            }
        }
        // Make sure to close stdin so gifski knows we're done
        drop(gifski_stdin);
        Ok(())
    });
    
    // Collect stderr for debugging
    let ffmpeg_stderr_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        ffmpeg_stderr.read_to_end(&mut buffer).await?;
        Ok::<String, std::io::Error>(String::from_utf8_lossy(&buffer).to_string())
    });
    
    let gifski_stderr_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        gifski_stderr.read_to_end(&mut buffer).await?;
        Ok::<String, std::io::Error>(String::from_utf8_lossy(&buffer).to_string())
    });
    
    // Wait for the pipe task with a timeout
    match tokio::time::timeout(Duration::from_secs(30), pipe_task).await {
        Ok(result) => {
            if let Err(e) = result? {
                return Err(anyhow!("Failed during FFmpeg to gifski piping: {}", e));
            }
        },
        Err(_) => return Err(anyhow!("Timeout while piping data between processes")),
    }
    
    // Wait for processes to complete
    let ffmpeg_status = ffmpeg_process.wait().await?;
    let gifski_status = gifski_process.wait().await?;
    
    // Check stderr outputs if processes failed
    if !ffmpeg_status.success() {
        let stderr = ffmpeg_stderr_task.await??;
        return Err(anyhow!("FFmpeg process failed: {}", stderr));
    }
    
    if !gifski_status.success() {
        let stderr = gifski_stderr_task.await??;
        return Err(anyhow!("gifski process failed: {}", stderr));
    }
    
    // Read the generated GIF file
    let mut gif_data = Vec::new();
    gifski_process.stdout.take().unwrap().read_to_end(&mut gif_data).await?;

    Ok(Bytes::from(gif_data))
}
