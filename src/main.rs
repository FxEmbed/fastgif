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
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt},
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
            "--output", "/dev/stdout",       // Output to stdout
            "-"                    // Read from stdin
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
    // Take ownership of the handles
    let mut gifski_stdin = gifski_process.stdin.take()
        .ok_or_else(|| anyhow!("Failed to take gifski stdin"))?;
    let mut ffmpeg_stdout = ffmpeg_process.stdout.take()
        .ok_or_else(|| anyhow!("Failed to take ffmpeg stdout"))?;
    let mut gifski_stdout = gifski_process.stdout.take()
        .ok_or_else(|| anyhow!("Failed to take gifski stdout"))?;
    let mut ffmpeg_stderr = ffmpeg_process.stderr.take()
        .ok_or_else(|| anyhow!("Failed to take ffmpeg stderr"))?;
    let mut gifski_stderr = gifski_process.stderr.take()
        .ok_or_else(|| anyhow!("Failed to take gifski stderr"))?;
    
    // --- Asynchronous Piping and Error Handling ---

    // Task to pipe ffmpeg stdout to gifski stdin
    let pipe_handle = tokio::spawn(async move {
        info!("Starting pipe: ffmpeg stdout -> gifski stdin");
        match tokio::io::copy(&mut ffmpeg_stdout, &mut gifski_stdin).await {
            Ok(bytes_copied) => {
                info!("Successfully piped {} bytes from ffmpeg to gifski", bytes_copied);
                // Explicitly close gifski's stdin by dropping the handle after copying is done.
                drop(gifski_stdin);
                Ok(())
            }
            Err(e) => {
                error!("Error piping data: {}", e);
                Err(anyhow!("Failed to pipe data from ffmpeg to gifski: {}", e))
            }
        }
    });

    // Task to read gifski stdout (the final GIF data)
    let collect_handle = tokio::spawn(async move {
        info!("Starting to collect gifski output");
        let mut gif_data = Vec::new();
        match gifski_stdout.read_to_end(&mut gif_data).await {
            Ok(_) => {
                info!("Collected {} bytes of GIF data from gifski", gif_data.len());
                Ok(gif_data)
            }
            Err(e) => {
                error!("Error reading gifski output: {}", e);
                Err(anyhow!("Failed to read gifski output: {}", e))
            }
        }
    });

    // Task to log ffmpeg stderr
    let ffmpeg_stderr_handle = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(ffmpeg_stderr);
        let mut line = String::new();
        info!("Monitoring ffmpeg stderr...");
        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
            info!("[ffmpeg stderr] {}", line.trim_end());
            line.clear();
        }
        info!("ffmpeg stderr stream finished.");
    });

    // Task to log gifski stderr
    let gifski_stderr_handle = tokio::spawn(async move {
        let mut reader = tokio::io::BufReader::new(gifski_stderr);
        let mut line = String::new();
        info!("Monitoring gifski stderr...");
        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
            info!("[gifski stderr] {}", line.trim_end());
            line.clear();
        }
        info!("gifski stderr stream finished.");
    });


    // --- Wait for all operations ---

    // Wait for the piping to complete
    pipe_handle.await??; // Double '?' to handle JoinError and the inner Result
    info!("Pipe operation completed successfully.");

    // Wait for gifski to finish and collect its output
    let gif_data = collect_handle.await??;
    info!("GIF data collection completed successfully.");


    // Wait for the processes to exit and check their statuses
    let ffmpeg_status = ffmpeg_process.wait().await?;
    info!("ffmpeg process exited with status: {}", ffmpeg_status);
    if !ffmpeg_status.success() {
        // Stderr is now logged concurrently, but we still signal failure
        return Err(anyhow!("FFmpeg process failed with exit code: {:?}", ffmpeg_status.code()));
    }

    let gifski_status = gifski_process.wait().await?;
    info!("gifski process exited with status: {}", gifski_status);
    if !gifski_status.success() {
        // Stderr is now logged concurrently
        return Err(anyhow!("gifski process failed with exit code: {:?}", gifski_status.code()));
    }

    // Wait for stderr logging tasks to finish (optional, but good practice)
    ffmpeg_stderr_handle.await?;
    gifski_stderr_handle.await?;
    info!("Stderr monitoring tasks finished.");

    info!("Successfully generated GIF with {} bytes", gif_data.len());
    Ok(Bytes::from(gif_data))
}
