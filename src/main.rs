use anyhow::{anyhow, Result, Error};
use axum::{
    extract::Path,
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt},
    process::Command as TokioCommand,
};
use tower_http::trace::TraceLayer;
use tracing::{error, info, Level};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with a pretty format
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_ansi(true)
        .init();

    info!("Starting FastGIF server");

    // Read port from environment variable or use default
    let port = env::var("PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(3000);

    // Our router
    let app = Router::new()
        .route("/tweet_video/{path}", get(handle_tweet_video))
        .fallback(handle_not_found)
        .layer(TraceLayer::new_for_http());

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

// Define the 404 handler function
async fn handle_not_found(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("404 Not Found: {}", uri))
}

async fn handle_tweet_video(Path(path): Path<String>) -> Response {
    info!("Processing video: {}", path);
    
    match process_tweet_video(&path).await {
        Ok(gif_data) => {
            info!("Successfully converted video to GIF ({} bytes)", gif_data.len());
            (
                StatusCode::OK,
                [
                    ("Content-Type", "image/gif"),
                    ("X-Powered-By", "fastgif"),
                    ("Cache-Control", "public, max-age=31536000")
                ],
                gif_data,
            )
                .into_response()
        }
        Err(e) => {
            error!("Failed to process video: {}", e);
            let error_message = format!("Failed to process video: {}\n\nStack trace:\n{}", 
                e, e.chain().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                error_message,
            )
                .into_response()
        }
    }
}

async fn process_tweet_video(path: &str) -> Result<Bytes> {
    let video_url = format!("https://video.twimg.com/tweet_video/{}", path);
    info!("Processing video from {}", video_url);

    // Set up FFmpeg process to read directly from the URL and output yuv4mpegpipe
    let mut ffmpeg_process = TokioCommand::new("ffmpeg")
        .args([
            "-i", &video_url,        // Read directly from URL
            "-f", "yuv4mpegpipe",   // Output in yuv4mpegpipe format
            "-"                     // Output to stdout
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn ffmpeg process: {}", e))?;
    
    // Set up gifski process to read yuv4mpegpipe frames from stdin and output to stdout
    let mut gifski_process = TokioCommand::new("gifski")
        .args([
            "--output", "-", 
            "--fast",
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
    let ffmpeg_stderr = ffmpeg_process.stderr.take()
        .ok_or_else(|| anyhow!("Failed to take ffmpeg stderr"))?;
    let gifski_stderr = gifski_process.stderr.take()
        .ok_or_else(|| anyhow!("Failed to take gifski stderr"))?;
    
    // --- Asynchronous Piping and Error Handling ---

    // Task to pipe ffmpeg stdout to gifski stdin
    let pipe_handle = tokio::spawn(async move {
        info!("Starting pipe: ffmpeg stdout -> gifski stdin");
        match tokio::io::copy(&mut ffmpeg_stdout, &mut gifski_stdin).await {
            Ok(bytes_copied) => {
                info!("Successfully piped {} bytes from ffmpeg to gifski", bytes_copied);
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
    // Spawned concurrently with the pipe_handle
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

    // Wait for the piping and collection tasks to complete.
    // It's often better to wait for results before waiting for process exit,
    // especially if process exit status depends on pipes being fully read/closed.
    let pipe_result = pipe_handle.await?;
    let collect_result = collect_handle.await?;

    // Check results from tasks first
    pipe_result?; // Propagate error from piping
    let gif_data = collect_result?; // Propagate error from collection & get data
    info!("Pipe and collect tasks completed successfully.");

    // Now, wait for the processes to exit and check their statuses.
    let ffmpeg_status = ffmpeg_process.wait().await
        .map_err(|e| anyhow!("Failed to wait for ffmpeg process: {}", e))?;
    info!("ffmpeg process exited with status: {}", ffmpeg_status);
    if !ffmpeg_status.success() {
        return Err(anyhow!("FFmpeg process failed with exit code: {:?}", ffmpeg_status.code()));
    }

    let gifski_status = gifski_process.wait().await
        .map_err(|e| anyhow!("Failed to wait for gifski process: {}", e))?;
    info!("gifski process exited with status: {}", gifski_status);
    if !gifski_status.success() {
        return Err(anyhow!("gifski process failed with exit code: {:?}", gifski_status.code()));
    }
    info!("ffmpeg and gifski processes completed successfully.");

    // Wait for stderr logging tasks to finish.
    ffmpeg_stderr_handle.await
        .map_err(|e| anyhow!("Failed to wait for ffmpeg stderr task: {}", e))?;
    gifski_stderr_handle.await
        .map_err(|e| anyhow!("Failed to wait for gifski stderr task: {}", e))?;
    info!("Stderr monitoring tasks finished.");

    info!("Successfully generated GIF with {} bytes", gif_data.len());
    Ok(Bytes::from(gif_data))
}
