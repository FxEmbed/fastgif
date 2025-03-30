# FastGIF

A Rust web server that converts Twitter videos to GIFs on the fly. This service:
- Takes Twitter video URLs through a simple API
- Downloads and processes videos using FFmpeg
- Converts videos to GIFs using gifski
- Streams the result back to the client

## Requirements

- Rust (latest stable version)
- FFmpeg (must be installed and available in PATH)
- gifski (must be installed and available in PATH)

## Installation

### Install Dependencies

#### FFmpeg

**macOS:**
```bash
brew install ffmpeg
```

**Ubuntu/Debian:**
```bash
sudo apt update
sudo apt install ffmpeg
```

#### gifski

**macOS:**
```bash
brew install gifski
```

**Ubuntu/Debian:**
```bash
cargo install gifski
```

### Build and Run

1. Clone the repository
2. Build the project:
```bash
cargo build --release
```
3. Run the server:
```bash
cargo run --release
```

The server will start on http://localhost:3000

## Usage

To convert a Twitter video to GIF, make a GET request to:

```
http://localhost:3000/{twitter_video_path}
```

For example, if the original Twitter video URL is:
```
https://video.twimg.com/tweet_video/FfyEjQ_WIAAd7rg.mp4
```

You would request:
```
http://localhost:3000/tweet_video/FfyEjQ_WIAAd7rg.mp4
```

The server will respond with a GIF of the video.

## Performance

This service uses streaming where possible to minimize latency:
- FFmpeg streams video data directly to gifski
- The resulting GIF is sent back to the client as soon as it's ready

## Configuration

The server runs on port 3000 by default. 